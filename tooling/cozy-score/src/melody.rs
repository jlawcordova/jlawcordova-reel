//! Melody generator: phrases built from 1-bar rhythm cells, chord tones on
//! strong beats, mostly stepwise motion, A A' phrase pairs.

use crate::harmony::{chord_tones, in_scale, Chord};
use crate::rng::Rng;

/// A melody note; `beat` counts from the first bar of the melody.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MelNote {
    pub beat: f64,
    pub len: f64,
    pub midi: i32,
}

/// 1-bar rhythm cells (in beats).
pub const CELLS: [&[f64]; 8] = [
    &[1.0, 1.0, 1.5, 0.5],
    &[1.0, 0.5, 0.5, 2.0],
    &[1.5, 0.5, 2.0],
    &[1.0, 1.0, 1.0, 1.0],
    &[2.0, 1.0, 1.0],
    &[1.0, 0.5, 0.5, 1.0, 1.0],
    &[1.5, 0.5, 1.0, 1.0],
    &[4.0],
];

/// Cells that end a phrase on a held note.
pub const ENDING_CELLS: [&[f64]; 4] =
    [&[1.0, 0.5, 0.5, 2.0], &[1.5, 0.5, 2.0], &[2.0, 2.0], &[4.0]];

/// Largest interval allowed (a major sixth).
pub const MAX_LEAP: i32 = 9;
/// Intervals of this size or more count as leaps and need a step back.
pub const LEAP: i32 = 5;

pub fn is_strong(pos_in_bar: f64) -> bool {
    pos_in_bar == 0.0 || pos_in_bar == 2.0
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    beat: f64,
    len: f64,
    chord: Chord,
    must_chord: bool,
    end_on_root: bool,
    prefer: Option<i32>,
}

struct Search<'a> {
    slots: &'a [Slot],
    lo: i32,
    hi: i32,
    rng: &'a mut Rng,
    budget: u32,
}

impl Search<'_> {
    fn candidates(&mut self, i: usize, prev: Option<i32>, prev_iv: i32, repeats: u32) -> Vec<i32> {
        let s = self.slots[i];
        let pool: Vec<i32> = if s.must_chord {
            chord_tones(s.chord, self.lo, self.hi)
        } else {
            (self.lo..=self.hi).filter(|&m| in_scale(m)).collect()
        };
        let mut ok: Vec<(i32, f64)> = Vec::new();
        for m in pool {
            if s.end_on_root && m.rem_euclid(12) != s.chord.root_pc() {
                continue;
            }
            let w = match prev {
                None => {
                    // Start mid-range.
                    let d = (m - 79).abs();
                    if d <= 4 {
                        4.0
                    } else {
                        1.0
                    }
                }
                Some(p) => {
                    let iv = m - p;
                    let a = iv.abs();
                    if a > MAX_LEAP {
                        continue;
                    }
                    if prev_iv.abs() >= LEAP && (a > 2 || a == 0 || iv.signum() == prev_iv.signum())
                    {
                        continue;
                    }
                    if a == 0 && repeats >= 1 {
                        continue;
                    }
                    // The phrase-final note is approached without a leap, so
                    // the next phrase is never forced into a step back.
                    if i + 1 == self.slots.len() && a >= LEAP {
                        continue;
                    }
                    match a {
                        0 => 1.0,
                        1 | 2 => 8.0,
                        3 | 4 => 3.0,
                        _ => 1.0,
                    }
                }
            };
            ok.push((m, w));
        }
        // Weighted shuffle; a preferred pitch (from the phrase being varied)
        // goes first when it is allowed.
        let mut order = Vec::with_capacity(ok.len());
        if let Some(p) = s.prefer {
            if let Some(ix) = ok.iter().position(|(m, _)| *m == p) {
                order.push(ok.remove(ix).0);
            }
        }
        while !ok.is_empty() {
            let ws: Vec<f64> = ok.iter().map(|x| x.1).collect();
            let k = self.rng.weighted(&ws);
            order.push(ok.remove(k).0);
        }
        order
    }

    fn run(
        &mut self,
        i: usize,
        prev: Option<i32>,
        prev_iv: i32,
        repeats: u32,
        out: &mut Vec<i32>,
    ) -> bool {
        if i == self.slots.len() {
            return true;
        }
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        for m in self.candidates(i, prev, prev_iv, repeats) {
            let iv = prev.map_or(0, |p| m - p);
            let rep = if iv == 0 && prev.is_some() {
                repeats + 1
            } else {
                0
            };
            out.push(m);
            if self.run(i + 1, Some(m), iv, rep, out) {
                return true;
            }
            out.pop();
        }
        false
    }
}

fn pick_cells(rng: &mut Rng, bars: usize) -> Vec<&'static [f64]> {
    (0..bars)
        .map(|b| {
            if b + 1 == bars {
                *rng.pick(&ENDING_CELLS)
            } else {
                *rng.pick(&CELLS)
            }
        })
        .collect()
}

fn slots_for(cells: &[&[f64]], chords: &[Chord], first_bar: usize) -> Vec<Slot> {
    let mut slots = Vec::new();
    for (b, cell) in cells.iter().enumerate() {
        let chord = chords[first_bar + b];
        let mut pos = 0.0;
        for (k, &len) in cell.iter().enumerate() {
            let last = b + 1 == cells.len() && k + 1 == cell.len();
            slots.push(Slot {
                beat: ((first_bar + b) * 4) as f64 + pos,
                len,
                chord,
                must_chord: is_strong(pos) || last,
                end_on_root: false,
                prefer: None,
            });
            pos += len;
        }
    }
    slots
}

/// Generates a melody over `chords` (one per bar) within [lo, hi].
///
/// Phrases are 4+4 bars when there are at least 8 bars, otherwise 2+2; each
/// pair is an idea and a varied repeat that keeps the rhythm and, where the
/// harmony allows, the first half of the pitches.
pub fn generate(
    rng: &mut Rng,
    chords: &[Chord],
    lo: i32,
    hi: i32,
    end_on_root: bool,
) -> Vec<MelNote> {
    let bars = chords.len();
    if bars == 0 {
        return Vec::new();
    }
    let plen = if bars >= 8 { 4 } else { 2.min(bars) };
    // Lay out phrases: (slots, index of the phrase this one varies).
    let mut phrases: Vec<(Vec<Slot>, Option<usize>)> = Vec::new();
    let mut bar = 0;
    while bar < bars {
        let a_len = plen.min(bars - bar);
        let cells = pick_cells(rng, a_len);
        phrases.push((slots_for(&cells, chords, bar), None));
        let a_ix = phrases.len() - 1;
        bar += a_len;
        if bar + a_len <= bars {
            phrases.push((slots_for(&cells, chords, bar), Some(a_ix)));
            bar += a_len;
        }
    }
    if end_on_root {
        if let Some(last) = phrases.last_mut().and_then(|p| p.0.last_mut()) {
            last.end_on_root = true;
        }
    }
    let mut realised: Vec<Vec<i32>> = Vec::new();
    let mut out: Vec<i32> = Vec::new();
    for (slots, varies) in &phrases {
        let mut run = slots.clone();
        if let Some(src) = varies {
            // A': same rhythm; keep A's pitches in the first half where the
            // harmony allows, and write a new second half.
            let first = run[0].beat;
            let half = (run.last().map_or(0.0, |s| s.beat + s.len) - first) / 2.0;
            for (k, s) in run.iter_mut().enumerate() {
                if s.beat - first < half {
                    s.prefer = realised[*src].get(k).copied();
                }
            }
        }
        let prev = out.last().copied();
        let prev_iv = if out.len() >= 2 {
            out[out.len() - 1] - out[out.len() - 2]
        } else {
            0
        };
        let mut part = Vec::new();
        let mut se = Search {
            slots: &run,
            lo,
            hi,
            rng,
            budget: 50_000,
        };
        let found = se.run(0, prev, prev_iv, 0, &mut part);
        if !found {
            part.clear();
            // Fall back to chord roots; never fails.
            part = run
                .iter()
                .map(|s| {
                    chord_tones(s.chord, lo, hi)
                        .into_iter()
                        .find(|m| m.rem_euclid(12) == s.chord.root_pc())
                        .unwrap_or(lo)
                })
                .collect();
        }
        out.extend(&part);
        realised.push(part);
    }
    phrases
        .iter()
        .flat_map(|p| p.0.iter())
        .zip(out)
        .map(|(s, m)| MelNote {
            beat: s.beat,
            len: s.len,
            midi: m,
        })
        .collect()
}

/// A 3-note motif reply over `chord`, starting at `beat` (eighths, then a
/// held note).
pub fn motif_reply(rng: &mut Rng, chord: Chord, beat: f64) -> Vec<MelNote> {
    let tones = chord_tones(chord, 76, 86);
    let start = *rng.pick(&tones);
    let shape: [i32; 3] = *rng.pick(&[[0, 2, 0], [0, -1, -3], [0, 2, 4], [0, -2, 0], [0, 1, 0]]);
    let mut notes = Vec::new();
    let mut m = start;
    let lens = [0.5, 0.5, 1.0];
    let mut b = beat;
    for (k, step) in shape.iter().enumerate() {
        let target = start + step;
        // Snap each target to the scale (or the chord, for the last note).
        m = nearest(target, |x| {
            if k == 2 {
                chord.is_chord_tone(x)
            } else {
                in_scale(x)
            }
        });
        notes.push(MelNote {
            beat: b,
            len: lens[k],
            midi: m,
        });
        b += lens[k];
    }
    let _ = m;
    notes
}

fn nearest(target: i32, ok: impl Fn(i32) -> bool) -> i32 {
    for d in 0..12 {
        if ok(target - d) {
            return target - d;
        }
        if ok(target + d) {
            return target + d;
        }
    }
    target
}

/// Long soft notes: one chord tone held per selected bar.
pub fn long_notes(rng: &mut Rng, chords: &[Chord], every: usize) -> Vec<MelNote> {
    let mut out = Vec::new();
    let mut prev = 78;
    for (b, &c) in chords.iter().enumerate() {
        if b % every != every - 1 {
            continue;
        }
        let tones: Vec<i32> = chord_tones(c, 74, 83)
            .into_iter()
            .filter(|m| (m - prev).abs() <= 5)
            .collect();
        let tones = if tones.is_empty() {
            chord_tones(c, 74, 83)
        } else {
            tones
        };
        let m = *rng.pick(&tones);
        let len = *rng.pick(&[2.0, 3.0, 4.0]);
        out.push(MelNote {
            beat: (b * 4) as f64,
            len,
            midi: m,
        });
        prev = m;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harmony::{chords_for, chords_with_ending, Loop};

    fn check(notes: &[MelNote], chords: &[Chord], lo: i32, hi: i32) {
        for n in notes {
            assert!((lo..=hi).contains(&n.midi), "out of range {n:?}");
            assert!(in_scale(n.midi), "out of scale {n:?}");
            let bar = (n.beat / 4.0).floor() as usize;
            let pos = n.beat - (bar * 4) as f64;
            if is_strong(pos) {
                assert!(
                    chords[bar].is_chord_tone(n.midi),
                    "strong beat not chord tone {n:?} over {:?}",
                    chords[bar]
                );
            }
        }
        for w in notes.windows(2) {
            assert!(
                (w[1].midi - w[0].midi).abs() <= MAX_LEAP,
                "leap too big {w:?}"
            );
        }
        for w in notes.windows(3) {
            let a = w[1].midi - w[0].midi;
            let b = w[2].midi - w[1].midi;
            if a.abs() >= LEAP {
                assert!(
                    b.signum() == -a.signum() && b.abs() <= 2,
                    "leap without step back {w:?}"
                );
            }
        }
        // Bars are filled exactly.
        let total: f64 = notes.iter().map(|n| n.len).sum();
        assert_eq!(total, (chords.len() * 4) as f64);
        // The phrase ends on a held chord tone.
        let last = notes.last().unwrap();
        assert!(last.len >= 2.0);
        assert!(chords.last().unwrap().is_chord_tone(last.midi));
    }

    #[test]
    fn melody_constraints_hold_across_seeds() {
        for seed in 0..40u64 {
            for lp in [Loop::DGBmA, Loop::DAcsGA, Loop::BmGDA, Loop::GDfsEmA] {
                for bars in [1usize, 2, 3, 4, 6, 8, 11] {
                    let chords = chords_for(lp, bars);
                    let mut r = Rng::new(seed).fork(&format!("{lp:?}{bars}"));
                    let notes = generate(&mut r, &chords, 74, 88, false);
                    check(&notes, &chords, 74, 88);
                }
            }
        }
    }

    #[test]
    fn ending_on_root() {
        for seed in 0..20u64 {
            let chords = chords_with_ending(Loop::DGBmA, 6);
            let notes = generate(&mut Rng::new(seed), &chords, 74, 88, true);
            check(&notes, &chords, 74, 88);
            assert_eq!(notes.last().unwrap().midi.rem_euclid(12), 2);
        }
    }

    #[test]
    fn varied_repeat_shares_rhythm() {
        let chords = chords_for(Loop::DGBmA, 8);
        let notes = generate(&mut Rng::new(7), &chords, 74, 88, false);
        let a: Vec<f64> = notes
            .iter()
            .filter(|n| n.beat < 16.0)
            .map(|n| n.beat)
            .collect();
        let b: Vec<f64> = notes
            .iter()
            .filter(|n| n.beat >= 16.0)
            .map(|n| n.beat - 16.0)
            .collect();
        assert_eq!(a, b);
        // Same chords under both halves, so the first two bars repeat.
        let a2: Vec<i32> = notes
            .iter()
            .filter(|n| n.beat < 8.0)
            .map(|n| n.midi)
            .collect();
        let b2: Vec<i32> = notes
            .iter()
            .filter(|n| (16.0..24.0).contains(&n.beat))
            .map(|n| n.midi)
            .collect();
        let same = a2.iter().zip(&b2).filter(|(x, y)| x == y).count();
        assert!(same * 2 >= a2.len(), "{a2:?} vs {b2:?}");
    }

    #[test]
    fn deterministic() {
        let chords = chords_for(Loop::BmGDA, 8);
        let a = generate(&mut Rng::new(9), &chords, 74, 88, false);
        let b = generate(&mut Rng::new(9), &chords, 74, 88, false);
        assert_eq!(a, b);
    }

    #[test]
    fn replies_and_long_notes() {
        let mut r = Rng::new(1);
        for c in [Chord::D, Chord::G, Chord::Bm, Chord::A] {
            let rep = motif_reply(&mut r, c, 2.0);
            assert_eq!(rep.len(), 3);
            assert!(c.is_chord_tone(rep[2].midi));
            assert!(rep.iter().all(|n| in_scale(n.midi)));
        }
        let chords = chords_for(Loop::DGBmA, 8);
        let ln = long_notes(&mut r, &chords, 2);
        assert_eq!(ln.len(), 4);
    }
}
