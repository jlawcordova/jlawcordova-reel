//! D major harmony: chords, voicings and chord loops.

use crate::rng::Rng;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Chord {
    D,
    G,
    Bm,
    A,
    Em,
    #[serde(rename = "A/C#")]
    AoverCs,
    #[serde(rename = "D/F#")]
    DoverFs,
}

/// MIDI numbers for one chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Voicing {
    pub bass: i32,
    pub pad: [i32; 3],
    /// Guitar arpeggio tones before the octave shift.
    pub arpeggio: [i32; 4],
}

impl Chord {
    pub fn name(self) -> &'static str {
        match self {
            Chord::D => "D",
            Chord::G => "G",
            Chord::Bm => "Bm",
            Chord::A => "A",
            Chord::Em => "Em",
            Chord::AoverCs => "A/C#",
            Chord::DoverFs => "D/F#",
        }
    }

    pub fn voicing(self) -> Voicing {
        let (bass, pad, arpeggio) = match self {
            Chord::D => (38, [62, 66, 69], [50, 57, 62, 66]),
            Chord::G => (43, [62, 67, 71], [55, 59, 62, 67]),
            Chord::Bm => (47, [62, 66, 71], [47, 54, 59, 62]),
            Chord::A => (45, [61, 64, 69], [45, 52, 57, 61]),
            Chord::Em => (40, [62, 67, 71], [52, 59, 64, 67]),
            Chord::AoverCs => (37, [61, 64, 69], [49, 57, 61, 64]),
            Chord::DoverFs => (42, [62, 66, 69], [54, 57, 62, 66]),
        };
        Voicing {
            bass,
            pad,
            arpeggio,
        }
    }

    /// Guitar arpeggio tones as played (one octave above the table).
    pub fn guitar_tones(self) -> [i32; 4] {
        self.voicing().arpeggio.map(|m| m + 12)
    }

    /// Pitch classes of the triad (0 = C).
    pub fn pitch_classes(self) -> [i32; 3] {
        match self {
            Chord::D | Chord::DoverFs => [2, 6, 9],
            Chord::G => [7, 11, 2],
            Chord::Bm => [11, 2, 6],
            Chord::A | Chord::AoverCs => [9, 1, 4],
            Chord::Em => [4, 7, 11],
        }
    }

    pub fn is_chord_tone(self, midi: i32) -> bool {
        self.pitch_classes().contains(&midi.rem_euclid(12))
    }

    /// Root pitch class.
    pub fn root_pc(self) -> i32 {
        self.pitch_classes()[0]
    }
}

/// D major scale pitch classes.
pub const SCALE: [i32; 7] = [2, 4, 6, 7, 9, 11, 1];

pub fn in_scale(midi: i32) -> bool {
    SCALE.contains(&midi.rem_euclid(12))
}

/// Scale tones within [lo, hi].
pub fn scale_tones(lo: i32, hi: i32) -> Vec<i32> {
    (lo..=hi).filter(|&m| in_scale(m)).collect()
}

/// Chord tones within [lo, hi].
pub fn chord_tones(chord: Chord, lo: i32, hi: i32) -> Vec<i32> {
    (lo..=hi).filter(|&m| chord.is_chord_tone(m)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loop {
    DGBmA,
    DAcsGA,
    BmGDA,
    /// Sponsor I-IV.
    DG,
    /// Reflect.
    GDfsEmA,
}

impl Loop {
    /// Loops a general section can draw from.
    pub const GENERAL: [Loop; 3] = [Loop::DGBmA, Loop::DAcsGA, Loop::BmGDA];

    pub fn chords(self) -> &'static [Chord] {
        use Chord::*;
        match self {
            Loop::DGBmA => &[D, G, Bm, A],
            Loop::DAcsGA => &[D, AoverCs, G, A],
            Loop::BmGDA => &[Bm, G, D, A],
            Loop::DG => &[D, G],
            Loop::GDfsEmA => &[G, DoverFs, Em, A],
        }
    }

    pub fn name(self) -> String {
        self.chords()
            .iter()
            .map(|c| c.name())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The ending cadence G A D (IV-V-I).
pub const ENDING: [Chord; 3] = [Chord::G, Chord::A, Chord::D];

/// Picks a general loop, avoiding the previous section's.
pub fn pick_loop(rng: &mut Rng, previous: Option<Loop>) -> Loop {
    let options: Vec<Loop> = Loop::GENERAL
        .iter()
        .copied()
        .filter(|l| Some(*l) != previous)
        .collect();
    *rng.pick(&options)
}

/// One chord per bar, cycling the loop.
pub fn chords_for(lp: Loop, bars: usize) -> Vec<Chord> {
    let c = lp.chords();
    (0..bars).map(|b| c[b % c.len()]).collect()
}

/// One chord per bar, ending with G A D on the last three bars.
pub fn chords_with_ending(lp: Loop, bars: usize) -> Vec<Chord> {
    let mut v = chords_for(lp, bars);
    let n = ENDING.len().min(bars);
    for i in 0..n {
        v[bars - n + i] = ENDING[ENDING.len() - n + i];
    }
    v
}

pub fn hz(m: f64) -> f64 {
    440.0 * libm::pow(2.0, (m - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voicing_table() {
        assert_eq!(Chord::D.voicing().bass, 38);
        assert_eq!(Chord::G.voicing().pad, [62, 67, 71]);
        assert_eq!(Chord::Bm.voicing().arpeggio, [47, 54, 59, 62]);
        assert_eq!(Chord::A.voicing().pad, [61, 64, 69]);
        assert_eq!(Chord::Em.voicing().bass, 40);
        assert_eq!(Chord::AoverCs.voicing().arpeggio, [49, 57, 61, 64]);
        assert_eq!(Chord::DoverFs.voicing().bass, 42);
        assert_eq!(Chord::D.guitar_tones(), [62, 69, 74, 78]);
    }

    #[test]
    fn voicings_are_chord_tones() {
        for c in [
            Chord::D,
            Chord::G,
            Chord::Bm,
            Chord::A,
            Chord::Em,
            Chord::AoverCs,
            Chord::DoverFs,
        ] {
            let v = c.voicing();
            assert!(c.is_chord_tone(v.bass), "{c:?}");
            // Pads may colour the chord (Em is voiced as Em7) but stay in key.
            assert!(v.arpeggio.iter().all(|&m| c.is_chord_tone(m)), "{c:?}");
            assert!(v.pad.iter().all(|&m| in_scale(m)));
        }
    }

    #[test]
    fn loops_and_endings() {
        assert_eq!(Loop::DGBmA.name(), "D G Bm A");
        assert_eq!(Loop::DAcsGA.name(), "D A/C# G A");
        assert_eq!(chords_for(Loop::DG, 3), vec![Chord::D, Chord::G, Chord::D]);
        assert_eq!(
            chords_with_ending(Loop::BmGDA, 5),
            vec![Chord::Bm, Chord::G, Chord::G, Chord::A, Chord::D]
        );
        assert_eq!(chords_with_ending(Loop::DGBmA, 2), vec![Chord::A, Chord::D]);
    }

    #[test]
    fn loop_pick_avoids_previous() {
        let mut r = Rng::new(3);
        for _ in 0..50 {
            assert_ne!(pick_loop(&mut r, Some(Loop::DGBmA)), Loop::DGBmA);
        }
    }

    #[test]
    fn hz_reference() {
        assert!((hz(69.0) - 440.0).abs() < 1e-9);
        assert!((hz(38.0) - 73.416).abs() < 1e-3);
    }
}
