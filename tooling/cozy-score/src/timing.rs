//! Tempo search and beat snapping.

use crate::types::Feel;

/// A section's place on the beat grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapped {
    pub start_beat: u32,
    pub end_beat: u32,
    /// Snapped times in seconds.
    pub start: f64,
    pub end: f64,
    /// Snapped minus requested, in seconds.
    pub start_offset: f64,
    pub end_offset: f64,
}

impl Snapped {
    pub fn beats(&self) -> u32 {
        self.end_beat - self.start_beat
    }
}

#[derive(Clone, Debug)]
pub struct Timing {
    pub bpm: f64,
    /// Seconds per beat.
    pub spb: f64,
    pub sections: Vec<Snapped>,
    /// Length of the audio: the plan's last end time, unsnapped.
    pub duration: f64,
}

impl Timing {
    pub fn beat_time(&self, beat: f64) -> f64 {
        beat * self.spb
    }
}

/// Error when no tempo in range gives every section at least one beat.
#[derive(Debug, PartialEq)]
pub struct TooShort(pub Vec<usize>);

fn snap_cost(bounds: &[f64], spb: f64) -> Option<f64> {
    let mut cost = 0.0;
    let mut prev_beat = 0i64;
    for &b in bounds {
        let beat = (b / spb).round() as i64;
        if beat <= prev_beat {
            return None;
        }
        let d = b - beat as f64 * spb;
        cost += d * d;
        prev_beat = beat;
    }
    Some(cost)
}

/// Picks the tempo (0.1 BPM steps within the feel's range) whose beat grid
/// best fits the section boundaries (least squared snap error; ties go to the
/// tempo nearest the middle of the range).
pub fn search_tempo(ends: &[f64], feel: Feel) -> Result<f64, TooShort> {
    let (lo, hi) = feel.bpm_range();
    let steps = ((hi - lo) * 10.0).round() as i64;
    let center = feel.bpm_center();
    let mut best: Option<(f64, f64)> = None;
    for i in 0..=steps {
        let bpm = (lo * 10.0 + i as f64) / 10.0;
        let spb = 60.0 / bpm;
        if let Some(c) = snap_cost(ends, spb) {
            let better = match best {
                None => true,
                Some((bc, bb)) => {
                    c < bc - 1e-12
                        || ((c - bc).abs() <= 1e-12 && (bpm - center).abs() < (bb - center).abs())
                }
            };
            if better {
                best = Some((c, bpm));
            }
        }
    }
    match best {
        Some((_, bpm)) => Ok(bpm),
        None => {
            // Report sections shorter than a beat at the slowest tempo.
            let spb = 60.0 / lo;
            let mut short = Vec::new();
            let mut prev = 0.0;
            for (i, &e) in ends.iter().enumerate() {
                if e - prev < spb {
                    short.push(i);
                }
                prev = e;
            }
            Err(TooShort(short))
        }
    }
}

/// A time the tempo search should fit, and how much it counts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub time: f64,
    pub weight: f64,
}

/// Weight of a `ticks` time in the tempo search; other event times count 1.
pub const TICK_WEIGHT: f64 = 4.0;

/// Distance at which a time stops pulling on the grid: the quantize
/// tolerance, since a time further off stays where it is anyway.
pub const GRID_CAP: f64 = 0.015;

/// Squared distance from the 1/16 grid at `bpm`, weighted, each distance
/// capped at [`GRID_CAP`].
pub fn grid_cost(anchors: &[Anchor], bpm: f64) -> f64 {
    let g = 15.0 / bpm;
    anchors
        .iter()
        .map(|a| {
            let d = (a.time - (a.time / g).round() * g).abs().min(GRID_CAP);
            a.weight * d * d
        })
        .sum()
}

/// Whether every section gets at least one beat at `bpm`.
pub fn fits_sections(ends: &[f64], bpm: f64) -> Result<(), TooShort> {
    let spb = 60.0 / bpm;
    if snap_cost(ends, spb).is_some() {
        return Ok(());
    }
    let mut short = Vec::new();
    let mut prev = 0.0;
    for (i, &e) in ends.iter().enumerate() {
        if e - prev < spb {
            short.push(i);
        }
        prev = e;
    }
    Err(TooShort(short))
}

/// How far a section boundary may move onto the beat before it costs as
/// much as an event time (half the distance that earns a warning).
pub const BOUNDARY_SLACK: f64 = 0.125;
/// Weight of a boundary's move within the slack.
pub const BOUNDARY_WEIGHT: f64 = 0.1;

/// How far the internal boundaries move when they snap to the beat at
/// `bpm`: squared at [`BOUNDARY_WEIGHT`], plus the part past
/// [`BOUNDARY_SLACK`] squared in full.
pub fn boundary_cost(ends: &[f64], bpm: f64) -> f64 {
    let spb = 60.0 / bpm;
    let inner = &ends[..ends.len().saturating_sub(1)];
    inner
        .iter()
        .map(|&b| {
            let d = (b - (b / spb).round() * spb).abs();
            let past = (d - BOUNDARY_SLACK).max(0.0);
            BOUNDARY_WEIGHT * d * d + past * past
        })
        .sum()
}

/// Picks the tempo (0.1 BPM steps within the feel's range) whose 1/16 grid
/// best fits the event times (see [`grid_cost`]) while moving the section
/// boundaries as little as it can onto the beat (see [`boundary_cost`]); ties go to the tempo nearest the middle of the
/// range. Every section still needs at least one beat. The last end is the
/// length of the audio and never moves.
pub fn search_tempo_events(ends: &[f64], events: &[Anchor], feel: Feel) -> Result<f64, TooShort> {
    let (lo, hi) = feel.bpm_range();
    let steps = ((hi - lo) * 10.0).round() as i64;
    let center = feel.bpm_center();
    let anchors = events;
    let mut best: Option<(f64, f64)> = None;
    for i in 0..=steps {
        let bpm = (lo * 10.0 + i as f64) / 10.0;
        if snap_cost(ends, 60.0 / bpm).is_none() {
            continue;
        }
        let c = grid_cost(anchors, bpm) + boundary_cost(ends, bpm);
        let better = match best {
            None => true,
            Some((bc, bb)) => {
                c < bc - 1e-12
                    || ((c - bc).abs() <= 1e-12 && (bpm - center).abs() < (bb - center).abs())
            }
        };
        if better {
            best = Some((c, bpm));
        }
    }
    match best {
        Some((_, bpm)) => Ok(bpm),
        None => fits_sections(ends, lo).map(|_| lo),
    }
}

/// A note value ("1/4", "1/8", "1/16", "1/32", "1/8t" triplet, "1/8." dotted)
/// as a number of beats.
pub fn note_value_beats(s: &str) -> Option<f64> {
    let t = s.trim();
    let (body, mult) = if let Some(b) = t.strip_suffix('t') {
        (b, 2.0 / 3.0)
    } else if let Some(b) = t.strip_suffix('.') {
        (b, 1.5)
    } else {
        (t, 1.0)
    };
    let d: u32 = body.strip_prefix("1/")?.parse().ok()?;
    if !matches!(d, 1 | 2 | 4 | 8 | 16 | 32 | 64) {
        return None;
    }
    Some(4.0 / d as f64 * mult)
}

/// Snaps the boundaries (section end times, in order) to the chosen tempo.
pub fn snap(ends: &[f64], bpm: f64) -> Timing {
    let spb = 60.0 / bpm;
    let mut sections = Vec::with_capacity(ends.len());
    let mut prev_beat = 0u32;
    let mut prev_req = 0.0;
    for &e in ends {
        let beat = ((e / spb).round() as u32).max(prev_beat + 1);
        let start = prev_beat as f64 * spb;
        let end = beat as f64 * spb;
        sections.push(Snapped {
            start_beat: prev_beat,
            end_beat: beat,
            start,
            end,
            start_offset: start - prev_req,
            end_offset: end - e,
        });
        prev_beat = beat;
        prev_req = e;
    }
    Timing {
        bpm,
        spb,
        sections,
        duration: ends.last().copied().unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_grid_is_found() {
        // 96 BPM: 0.625 s per beat. 5 s = 8 beats, 12.5 s = 20 beats.
        let bpm = search_tempo(&[5.0, 12.5, 20.0], Feel::Spring).unwrap();
        assert!((bpm - 96.0).abs() < 1e-9, "{bpm}");
        let t = snap(&[5.0, 12.5, 20.0], bpm);
        assert_eq!(t.sections[0].end_beat, 8);
        assert_eq!(t.sections[1].start_beat, 8);
        assert_eq!(t.sections[2].end_beat, 32);
        assert!(t.sections.iter().all(|s| s.end_offset.abs() < 1e-9));
    }

    #[test]
    fn summer_range() {
        // 120 BPM: 0.5 s per beat.
        let bpm = search_tempo(&[4.0, 10.0, 31.5], Feel::Summer).unwrap();
        assert!((116.0..=124.0).contains(&bpm));
        let t = snap(&[4.0, 10.0, 31.5], bpm);
        for s in &t.sections {
            assert!(s.end_offset.abs() < 0.26, "{s:?}");
        }
    }

    #[test]
    fn reference_plan_snaps_close() {
        let ends = [4.0, 12.0, 24.0, 40.0, 45.0, 52.0, 60.0];
        let bpm = search_tempo(&ends, Feel::Spring).unwrap();
        assert!((92.0..=100.0).contains(&bpm));
        let t = snap(&ends, bpm);
        let spb = 60.0 / bpm;
        for s in &t.sections {
            assert!(s.end_offset.abs() <= spb / 2.0 + 1e-9);
            assert!(s.beats() >= 1);
        }
        assert_eq!(t.duration, 60.0);
    }

    #[test]
    fn note_values() {
        assert_eq!(note_value_beats("1/16"), Some(0.25));
        assert_eq!(note_value_beats("1/8"), Some(0.5));
        assert_eq!(note_value_beats("1/32"), Some(0.125));
        assert!((note_value_beats("1/8t").unwrap() - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(note_value_beats("1/4."), Some(1.5));
        assert_eq!(note_value_beats("1/3"), None);
        assert_eq!(note_value_beats("0.1"), None);
        // 1/16 at 150 BPM is 0.1 s.
        assert!((note_value_beats("1/16").unwrap() * 60.0 / 150.0 - 0.1).abs() < 1e-12);
    }

    #[test]
    fn reference_ticks_choose_150() {
        let ticks = [1.4, 1.8, 2.2, 2.4, 2.8, 3.0, 3.4, 3.8, 4.4, 4.8, 5.4];
        let mut ev: Vec<Anchor> = ticks
            .iter()
            .map(|&time| Anchor {
                time,
                weight: TICK_WEIGHT,
            })
            .collect();
        ev.extend([0.2, 1.2, 5.95, 6.8].map(|time| Anchor { time, weight: 1.0 }));
        let bpm = search_tempo_events(&[1.2, 5.9, 7.0], &ev, Feel::Chip).unwrap();
        assert!((bpm - 150.0).abs() < 1e-9, "{bpm}");
    }

    #[test]
    fn events_never_drag_a_boundary_far_off_the_beat() {
        // 92 BPM puts 4 s 0.087 s from a beat; 97.5 would move it 0.308 s,
        // for a hit that fits a sixteenth slightly better.
        let bpm = search_tempo_events(
            &[4.0, 8.0],
            &[Anchor {
                time: 6.0,
                weight: 1.0,
            }],
            Feel::Spring,
        )
        .unwrap();
        let spb = 60.0 / bpm;
        let moved = (4.0 - (4.0 / spb).round() * spb).abs();
        assert!(moved <= BOUNDARY_SLACK, "{bpm}: {moved}");
    }

    #[test]
    fn too_short_sections_are_reported() {
        let err = search_tempo(&[0.1, 4.0], Feel::Spring).unwrap_err();
        assert_eq!(err, TooShort(vec![0]));
    }
}
