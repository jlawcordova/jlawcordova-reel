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
    fn too_short_sections_are_reported() {
        let err = search_tempo(&[0.1, 4.0], Feel::Spring).unwrap_err();
        assert_eq!(err, TooShort(vec![0]));
    }
}
