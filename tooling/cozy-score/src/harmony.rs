//! Harmony: chords, voicings, chord loops and keys.
//!
//! Every feel is written in its home key (D major for spring and summer,
//! C major for chip). `--key` transposes the whole score from there; see
//! [`key_shift`].

use crate::rng::Rng;
use crate::types::Feel;
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
    Cmaj7,
    Am7,
    Fmaj7,
    G7,
    Dm7,
    Em7,
    #[serde(rename = "C(add9)")]
    Cadd9,
}

/// MIDI numbers for one chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Voicing {
    pub bass: i32,
    pub pad: &'static [i32],
    /// Picking tones: the guitar's 4 (before its octave shift), or the
    /// 8-step synth-pluck walk of the chip feel.
    pub arpeggio: &'static [i32],
}

impl Chord {
    pub const ALL: [Chord; 14] = [
        Chord::D,
        Chord::G,
        Chord::Bm,
        Chord::A,
        Chord::Em,
        Chord::AoverCs,
        Chord::DoverFs,
        Chord::Cmaj7,
        Chord::Am7,
        Chord::Fmaj7,
        Chord::G7,
        Chord::Dm7,
        Chord::Em7,
        Chord::Cadd9,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Chord::D => "D",
            Chord::G => "G",
            Chord::Bm => "Bm",
            Chord::A => "A",
            Chord::Em => "Em",
            Chord::AoverCs => "A/C#",
            Chord::DoverFs => "D/F#",
            Chord::Cmaj7 => "Cmaj7",
            Chord::Am7 => "Am7",
            Chord::Fmaj7 => "Fmaj7",
            Chord::G7 => "G7",
            Chord::Dm7 => "Dm7",
            Chord::Em7 => "Em7",
            Chord::Cadd9 => "C(add9)",
        }
    }

    /// Root pitch class, the rest of the name, and a slash bass.
    fn spelling(self) -> (i32, &'static str, Option<i32>) {
        match self {
            Chord::D => (2, "", None),
            Chord::G => (7, "", None),
            Chord::Bm => (11, "m", None),
            Chord::A => (9, "", None),
            Chord::Em => (4, "m", None),
            Chord::AoverCs => (9, "", Some(1)),
            Chord::DoverFs => (2, "", Some(6)),
            Chord::Cmaj7 => (0, "maj7", None),
            Chord::Am7 => (9, "m7", None),
            Chord::Fmaj7 => (5, "maj7", None),
            Chord::G7 => (7, "7", None),
            Chord::Dm7 => (2, "m7", None),
            Chord::Em7 => (4, "m7", None),
            Chord::Cadd9 => (0, "(add9)", None),
        }
    }

    /// The chord's name after transposing by `shift` semitones.
    pub fn name_in(self, shift: i32, flats: bool) -> String {
        if shift.rem_euclid(12) == 0 {
            return self.name().to_string();
        }
        let (root, rest, slash) = self.spelling();
        let mut s = format!("{}{rest}", note_name(root + shift, flats));
        if let Some(b) = slash {
            s.push('/');
            s.push_str(note_name(b + shift, flats));
        }
        s
    }

    pub fn voicing(self) -> Voicing {
        let (bass, pad, arpeggio): (i32, &'static [i32], &'static [i32]) = match self {
            Chord::D => (38, &[62, 66, 69], &[50, 57, 62, 66]),
            Chord::G => (43, &[62, 67, 71], &[55, 59, 62, 67]),
            Chord::Bm => (47, &[62, 66, 71], &[47, 54, 59, 62]),
            Chord::A => (45, &[61, 64, 69], &[45, 52, 57, 61]),
            Chord::Em => (40, &[62, 67, 71], &[52, 59, 64, 67]),
            Chord::AoverCs => (37, &[61, 64, 69], &[49, 57, 61, 64]),
            Chord::DoverFs => (42, &[62, 66, 69], &[54, 57, 62, 66]),
            Chord::Cmaj7 => (36, &[60, 64, 67, 71], &[60, 64, 67, 71, 72, 71, 67, 64]),
            Chord::Am7 => (45, &[57, 60, 64, 67], &[57, 60, 64, 67, 69, 67, 64, 60]),
            Chord::Fmaj7 => (41, &[57, 60, 64, 65], &[53, 57, 60, 64, 65, 64, 60, 57]),
            Chord::G7 => (43, &[55, 59, 62, 65], &[55, 59, 62, 65, 67, 65, 62, 59]),
            Chord::Dm7 => (38, &[57, 60, 62, 65], &[50, 53, 57, 60, 62, 60, 57, 53]),
            Chord::Em7 => (40, &[55, 59, 62, 64], &[52, 55, 59, 62, 64, 62, 59, 55]),
            Chord::Cadd9 => (36, &[60, 62, 64, 67], &[60, 64, 67, 74, 72, 67, 64, 62]),
        };
        Voicing {
            bass,
            pad,
            arpeggio,
        }
    }

    /// Guitar arpeggio tones as played (one octave above the table).
    pub fn guitar_tones(self) -> Vec<i32> {
        self.voicing().arpeggio[..4]
            .iter()
            .map(|m| m + 12)
            .collect()
    }

    /// Pitch classes of the chord (0 = C), root first.
    pub fn pitch_classes(self) -> &'static [i32] {
        match self {
            Chord::D | Chord::DoverFs => &[2, 6, 9],
            Chord::G => &[7, 11, 2],
            Chord::Bm => &[11, 2, 6],
            Chord::A | Chord::AoverCs => &[9, 1, 4],
            Chord::Em => &[4, 7, 11],
            Chord::Cmaj7 => &[0, 4, 7, 11],
            Chord::Am7 => &[9, 0, 4, 7],
            Chord::Fmaj7 => &[5, 9, 0, 4],
            Chord::G7 => &[7, 11, 2, 5],
            Chord::Dm7 => &[2, 5, 9, 0],
            Chord::Em7 => &[4, 7, 11, 2],
            Chord::Cadd9 => &[0, 4, 7, 2],
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

const SHARPS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLATS: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

pub fn note_name(pc: i32, flats: bool) -> &'static str {
    let i = pc.rem_euclid(12) as usize;
    if flats {
        FLATS[i]
    } else {
        SHARPS[i]
    }
}

/// D major scale pitch classes.
pub const SCALE: [i32; 7] = [2, 4, 6, 7, 9, 11, 1];

/// Major-scale steps from the tonic.
pub const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Major-pentatonic steps from the tonic.
pub const PENTATONIC: [i32; 5] = [0, 2, 4, 7, 9];

pub fn in_scale(midi: i32) -> bool {
    SCALE.contains(&midi.rem_euclid(12))
}

/// Whether `midi` is in the major scale on `tonic` (a pitch class).
pub fn in_scale_of(tonic: i32, midi: i32) -> bool {
    MAJOR.contains(&(midi - tonic).rem_euclid(12))
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
    // The chip feel's loops, one chord per bar.
    CAmFG,
    FGEmAm,
    CFDmG,
    /// Chip sponsor I-IV.
    CF,
    /// Chip reflect.
    FCDmG,
}

impl Loop {
    /// Loops a general section can draw from.
    pub const GENERAL: [Loop; 3] = [Loop::DGBmA, Loop::DAcsGA, Loop::BmGDA];
    pub const CHIP: [Loop; 3] = [Loop::CAmFG, Loop::FGEmAm, Loop::CFDmG];

    pub fn chords(self) -> &'static [Chord] {
        use Chord::*;
        match self {
            Loop::DGBmA => &[D, G, Bm, A],
            Loop::DAcsGA => &[D, AoverCs, G, A],
            Loop::BmGDA => &[Bm, G, D, A],
            Loop::DG => &[D, G],
            Loop::GDfsEmA => &[G, DoverFs, Em, A],
            Loop::CAmFG => &[Cmaj7, Am7, Fmaj7, G7],
            Loop::FGEmAm => &[Fmaj7, G7, Em7, Am7],
            Loop::CFDmG => &[Cmaj7, Fmaj7, Dm7, G7],
            Loop::CF => &[Cmaj7, Fmaj7],
            Loop::FCDmG => &[Fmaj7, Cmaj7, Dm7, G7],
        }
    }

    pub fn name(self) -> String {
        self.name_in(0, false)
    }

    pub fn name_in(self, shift: i32, flats: bool) -> String {
        self.chords()
            .iter()
            .map(|c| c.name_in(shift, flats))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The ending cadence G A D (IV-V-I).
pub const ENDING: [Chord; 3] = [Chord::G, Chord::A, Chord::D];
/// The chip feel's ending: Fmaj7 G7 C(add9).
pub const CHIP_ENDING: [Chord; 3] = [Chord::Fmaj7, Chord::G7, Chord::Cadd9];

/// What a feel's harmony is built from, in its home key.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Home tonic pitch class.
    pub tonic: i32,
    /// Home tonic relative to D, for figures written in D (0 or -2).
    pub offset: i32,
    pub general: [Loop; 3],
    pub sponsor: Loop,
    pub reflect: Loop,
    pub tonic_chord: Chord,
    pub sub_chord: Chord,
    /// The chord a piece ends on.
    pub final_chord: Chord,
    pub ending: [Chord; 3],
    /// Lead range and the reveal's top note.
    pub mel_lo: i32,
    pub mel_hi: i32,
    pub reveal_hi: i32,
}

pub fn palette(feel: Feel) -> Palette {
    match feel {
        Feel::Spring | Feel::Summer => Palette {
            tonic: 2,
            offset: 0,
            general: Loop::GENERAL,
            sponsor: Loop::DG,
            reflect: Loop::GDfsEmA,
            tonic_chord: Chord::D,
            sub_chord: Chord::G,
            final_chord: Chord::D,
            ending: ENDING,
            mel_lo: 74,
            mel_hi: 88,
            reveal_hi: 90,
        },
        Feel::Chip => Palette {
            tonic: 0,
            offset: -2,
            general: Loop::CHIP,
            sponsor: Loop::CF,
            reflect: Loop::FCDmG,
            tonic_chord: Chord::Cmaj7,
            sub_chord: Chord::Fmaj7,
            final_chord: Chord::Cadd9,
            ending: CHIP_ENDING,
            mel_lo: 72,
            mel_hi: 88,
            reveal_hi: 90,
        },
    }
}

/// Picks a general loop, avoiding the previous section's.
pub fn pick_loop(rng: &mut Rng, previous: Option<Loop>) -> Loop {
    pick_loop_from(rng, previous, &Loop::GENERAL)
}

pub fn pick_loop_from(rng: &mut Rng, previous: Option<Loop>, general: &[Loop]) -> Loop {
    let options: Vec<Loop> = general
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
    chords_ending_in(lp, bars, &ENDING)
}

/// One chord per bar, ending with `ending` on the last bars.
pub fn chords_ending_in(lp: Loop, bars: usize, ending: &[Chord]) -> Vec<Chord> {
    let mut v = chords_for(lp, bars);
    let n = ending.len().min(bars);
    for i in 0..n {
        v[bars - n + i] = ending[ending.len() - n + i];
    }
    v
}

/// A major key given with `--key` or the plan's `key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub tonic: i32,
    /// Spell with flats (as typed, or the usual spelling of the key).
    pub flats: bool,
}

/// Parses "C", "C#", "Db", ... "B" (optionally followed by "major").
pub fn parse_key(s: &str) -> Result<Key, String> {
    let t = s.trim();
    let t = t
        .strip_suffix("major")
        .or_else(|| t.strip_suffix("maj"))
        .unwrap_or(t)
        .trim();
    let mut chars = t.chars();
    let letter = chars.next().map(|c| c.to_ascii_uppercase());
    let base: i32 = match letter {
        Some('C') => 0,
        Some('D') => 2,
        Some('E') => 4,
        Some('F') => 5,
        Some('G') => 7,
        Some('A') => 9,
        Some('B') => 11,
        _ => return Err(format!("key \"{s}\" must be a note name C, C#, Db, ... B")),
    };
    let rest: String = chars.collect();
    let (acc, flats) = match rest.as_str() {
        "" => (0, matches!(base, 5)),
        "#" | "♯" => (1, false),
        "b" | "♭" => (-1, true),
        "m" | "min" | "minor" => return Err(format!("key \"{s}\": major keys only for now")),
        _ => return Err(format!("key \"{s}\" must be a note name C, C#, Db, ... B")),
    };
    Ok(Key {
        tonic: (base + acc).rem_euclid(12),
        flats,
    })
}

/// The bass range every key keeps its voicing roots in.
pub const BASS_LO: i32 = 33;
pub const BASS_HI: i32 = 47;
/// The range every key keeps the lead in.
pub const LEAD_LO: i32 = 72;
pub const LEAD_HI: i32 = 90;

/// Moves a bass note by octaves into [BASS_LO, BASS_HI].
pub fn fold_bass(m: i32) -> i32 {
    let mut m = m;
    while m > BASS_HI {
        m -= 12;
    }
    while m < BASS_LO {
        m += 12;
    }
    m
}

/// Semitones from the feel's home key to `key`: the octave (up or down)
/// that leaves the most voicing roots between MIDI 33 and 47, then the
/// smaller move. The rest are folded by [`fold_bass`].
pub fn key_shift(feel: Feel, key: Option<Key>) -> i32 {
    let Some(key) = key else { return 0 };
    let home = palette(feel).tonic;
    let k = (key.tonic - home).rem_euclid(12);
    if k == 0 {
        return 0;
    }
    let roots: Vec<i32> = Chord::ALL
        .iter()
        .filter(|c| feel_chords(feel).contains(c))
        .map(|c| c.voicing().bass)
        .collect();
    let outside = |s: i32| {
        roots
            .iter()
            .filter(|&&b| !(BASS_LO..=BASS_HI).contains(&(b + s)))
            .count()
    };
    let (up, down) = (k, k - 12);
    match outside(up).cmp(&outside(down)) {
        std::cmp::Ordering::Less => up,
        std::cmp::Ordering::Greater => down,
        std::cmp::Ordering::Equal => {
            if up.abs() < down.abs() {
                up
            } else {
                down
            }
        }
    }
}

/// The chords a feel uses.
pub fn feel_chords(feel: Feel) -> Vec<Chord> {
    let p = palette(feel);
    let mut v: Vec<Chord> = p
        .general
        .iter()
        .chain([p.sponsor, p.reflect].iter())
        .flat_map(|l| l.chords().iter().copied())
        .chain(p.ending)
        .chain([p.tonic_chord, p.sub_chord, p.final_chord])
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The key's name after the shift, e.g. "D major".
pub fn key_name(feel: Feel, shift: i32, flats: bool) -> String {
    format!("{} major", note_name(palette(feel).tonic + shift, flats))
}

/// A lead range [lo, hi] in home-key terms, moved so that after `shift` it
/// sits inside [LEAD_LO, LEAD_HI] (narrowed only if it is wider).
pub fn lead_window(lo: i32, hi: i32, shift: i32) -> (i32, i32) {
    if shift == 0 {
        return (lo, hi);
    }
    let span = (hi - lo).min(LEAD_HI - LEAD_LO);
    let a = (lo + shift).clamp(LEAD_LO, LEAD_HI - span);
    (a - shift, a - shift + span)
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
    fn chip_voicings_are_in_c_major() {
        assert_eq!(Chord::Cmaj7.voicing().pad, [60, 64, 67, 71]);
        assert_eq!(
            Chord::Am7.voicing().arpeggio,
            [57, 60, 64, 67, 69, 67, 64, 60]
        );
        assert_eq!(Chord::Fmaj7.voicing().bass, 41);
        assert_eq!(Chord::G7.voicing().pad, [55, 59, 62, 65]);
        for c in feel_chords(Feel::Chip) {
            let v = c.voicing();
            assert!(c.is_chord_tone(v.bass), "{c:?}");
            assert!(v.pad.iter().all(|&m| c.is_chord_tone(m)), "{c:?}");
            assert!(v.arpeggio.iter().all(|&m| c.is_chord_tone(m)), "{c:?}");
            assert_eq!(v.arpeggio.len(), 8);
            assert!(v.pad.iter().all(|&m| in_scale_of(0, m)));
        }
        assert_eq!(Loop::CAmFG.name(), "Cmaj7 Am7 Fmaj7 G7");
        assert_eq!(
            chords_ending_in(Loop::CFDmG, 5, &CHIP_ENDING),
            vec![
                Chord::Cmaj7,
                Chord::Fmaj7,
                Chord::Fmaj7,
                Chord::G7,
                Chord::Cadd9
            ]
        );
    }

    #[test]
    fn keys_transpose_names_and_fold_the_bass() {
        assert_eq!(Chord::AoverCs.name_in(-2, false), "G/B");
        assert_eq!(Chord::Cmaj7.name_in(3, true), "Ebmaj7");
        assert_eq!(Chord::Cadd9.name_in(2, false), "D(add9)");
        assert_eq!(key_name(Feel::Chip, 0, false), "C major");
        assert_eq!(key_name(Feel::Spring, 0, false), "D major");
        assert_eq!(fold_bass(50), 38);
        assert_eq!(fold_bass(30), 42);
        for feel in [Feel::Spring, Feel::Chip] {
            for tonic in 0..12 {
                let s = key_shift(
                    feel,
                    Some(Key {
                        tonic,
                        flats: false,
                    }),
                );
                assert!((-11..=11).contains(&s));
                assert_eq!((palette(feel).tonic + s).rem_euclid(12), tonic);
                let (lo, hi) = lead_window(74, 90, s);
                assert!(lo + s >= LEAD_LO && hi + s <= LEAD_HI);
            }
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
