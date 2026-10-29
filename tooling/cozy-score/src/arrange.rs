//! Arrangement: turns the snapped plan into note events by role.
//!
//! Nothing here knows which instruments are allowed. Every random choice comes
//! from forks keyed by section and role, so the notes depend only on the plan,
//! feel and seed.
//!
//! Figures are written in the feel's home key (D major, or C major for chip);
//! a key shift is applied as notes are written (see [`Writer::add_at`]).

use crate::harmony::{
    chord_tones, chords_ending_in, chords_for, fold_bass, in_scale_of, lead_window, palette,
    pick_loop_from, Chord, Loop, Palette,
};
use crate::melody::{self, MelNote};
use crate::rng::Rng;
use crate::timing::Timing;
use crate::types::{Feel, Instrument, Role, VideoType};
use serde::Serialize;

/// How a note is played; each instrument interprets it in its own way.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Art {
    /// Instrument default.
    Normal,
    /// Picked 8th: guitar rings 1.4-1.5 s.
    Eighth,
    /// Picked quarter: guitar rings 2.2 s.
    Quarter,
    /// One string of a strum; guitar rings for `ring` seconds.
    Strum { ring: f64 },
    /// Short stab: guitar muted 0.25 s, mallet 0.5 s.
    Muted,
    /// Held for the written length (pads, held bass, long lead notes).
    Held,
    /// A soft note that glides up into its pitch (a floating entrance piece).
    Glide,
    /// No attack: the note swells in over 120 ms (a fading entrance piece).
    Swell,
    /// Played backwards in level: a falling riser.
    Reverse,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NoteEvent {
    /// Stable id: owner section, role and a per-role counter.
    pub id: u64,
    /// The section whose allow-list decides who plays this note.
    pub section: usize,
    pub role: Role,
    /// Start in seconds, humanised.
    pub time: f64,
    /// Written duration in seconds.
    pub dur: f64,
    pub midi: i32,
    /// Level relative to the role's nominal level.
    pub gain: f64,
    pub art: Art,
}

#[derive(Clone, Debug, Serialize)]
pub struct TransitionInfo {
    pub boundary: usize,
    pub time: f64,
    pub from: VideoType,
    pub to: VideoType,
    pub energy_delta: f64,
    /// "run-up", "pickup", "drop", "none", or "event" when a run event
    /// lands on the boundary instead.
    pub kind: &'static str,
    pub hit: bool,
    /// Section that owns (and whose instruments play) the transition.
    pub owner: usize,
}

#[derive(Clone, Debug)]
pub struct SectionPlan {
    pub index: usize,
    pub kind: VideoType,
    pub start: f64,
    pub end: f64,
    pub beats: u32,
    pub bars: usize,
    pub chords: Vec<Chord>,
    pub loop_name: String,
    pub preferred_bass: Instrument,
    pub feel: Feel,
}

impl SectionPlan {
    /// The chord sounding at absolute time `t` (inside or after the section).
    pub fn chord_at(&self, t: f64, spb: f64) -> Chord {
        let bar = ((t - self.start) / (4.0 * spb)).floor().max(0.0) as usize;
        self.chords[bar.min(self.chords.len() - 1)]
    }
}

#[derive(Clone, Debug)]
pub struct Arrangement {
    pub bpm: f64,
    pub spb: f64,
    pub duration: f64,
    pub feel: Feel,
    /// Semitones from the feel's home key.
    pub shift: i32,
    pub sections: Vec<SectionPlan>,
    pub events: Vec<NoteEvent>,
    pub transitions: Vec<TransitionInfo>,
}

impl Arrangement {
    pub fn events_without_lead(&self) -> Vec<NoteEvent> {
        self.events
            .iter()
            .filter(|e| e.role != Role::Lead)
            .cloned()
            .collect()
    }

    /// The section that plays at time `t` (the last one past the end).
    pub fn section_at(&self, t: f64) -> usize {
        self.sections
            .iter()
            .rposition(|s| t >= s.start - 1e-9)
            .unwrap_or(0)
    }

    pub fn chord_at(&self, t: f64) -> Chord {
        self.sections[self.section_at(t)].chord_at(t, self.spb)
    }
}

/// Melody range.
pub const MEL_LO: i32 = 74;
pub const MEL_HI: i32 = 88;
/// The reveal may climb to F#6.
pub const REVEAL_HI: i32 = 90;

fn jitter_sigma(role: Role) -> f64 {
    match role {
        Role::Kick => 0.003,
        Role::Shaker | Role::Backbeat | Role::Bass => 0.004,
        Role::Picking | Role::ChordHits | Role::Fx => 0.005,
        Role::Motif => 0.006,
        Role::Lead | Role::Pad | Role::Riser => 0.007,
    }
}

pub fn preferred_bass(feel: Feel, kind: VideoType) -> Instrument {
    if feel != Feel::Spring
        || matches!(
            kind,
            VideoType::Sponsor | VideoType::RecapCta | VideoType::Reflect
        )
    {
        Instrument::SoftBass
    } else {
        Instrument::HeavyBass
    }
}

/// Roles whose notes follow the key. Drums and noise carry placeholder
/// pitches, and bass roots are transposed (and folded) where they are chosen.
pub fn follows_key(role: Role) -> bool {
    !matches!(
        role,
        Role::Kick | Role::Shaker | Role::Backbeat | Role::Riser | Role::Bass
    )
}

/// The feel, its harmony and the key shift, shared by every writer.
#[derive(Clone, Copy, Debug)]
struct Ctx {
    feel: Feel,
    pal: Palette,
    shift: i32,
}

impl Ctx {
    /// A chord's bass root in the target key, folded into MIDI 33-47.
    fn bass(&self, chord: Chord) -> i32 {
        let b = chord.voicing().bass;
        if self.shift == 0 {
            b
        } else {
            fold_bass(b + self.shift)
        }
    }

    fn chip(&self) -> bool {
        self.feel == Feel::Chip
    }

    /// A lead range from the D major tables, in home-key terms.
    fn lead(&self, lo: i32, hi: i32) -> (i32, i32) {
        lead_window(lo + self.pal.offset, hi + self.pal.offset, self.shift)
    }

    fn in_scale(&self, m: i32) -> bool {
        in_scale_of(self.pal.tonic, m)
    }
}

/// Adds events for one owner section.
struct Writer<'a> {
    sec: usize,
    start: f64,
    spb: f64,
    /// Beats in the section; events at or after this are cut.
    beats: f64,
    rng: Rng,
    hum: Rng,
    counters: [u32; Role::COUNT],
    /// Pad chords requested per bar; tied and written by `flush_pads`.
    pads: Vec<(usize, Chord, f64)>,
    cx: Ctx,
    out: &'a mut Vec<NoteEvent>,
}

impl<'a> Writer<'a> {
    fn new(
        sec: usize,
        start: f64,
        spb: f64,
        beats: f64,
        root: &Rng,
        cx: Ctx,
        out: &'a mut Vec<NoteEvent>,
    ) -> Self {
        let r = root.fork_n(1000 + sec as u64);
        Writer {
            sec,
            start,
            spb,
            beats,
            rng: r.fork("parts"),
            hum: r.fork("humanize"),
            counters: [0; Role::COUNT],
            pads: Vec::new(),
            cx,
            out,
        }
    }

    fn next_id(&mut self, role: Role) -> u64 {
        let r = role as usize;
        let c = self.counters[r];
        self.counters[r] += 1;
        ((self.sec as u64) << 40) | ((r as u64) << 32) | u64::from(c)
    }

    /// Absolute-time event (transitions); not cut to the section. Pitched
    /// notes move by the key shift here.
    fn add_at(&mut self, role: Role, time: f64, dur: f64, midi: i32, gain: f64, art: Art) {
        let sigma = jitter_sigma(role);
        let j = (self.hum.gauss() * sigma).clamp(-2.5 * sigma, 2.5 * sigma);
        let id = self.next_id(role);
        let midi = if follows_key(role) {
            midi + self.cx.shift
        } else {
            midi
        };
        self.out.push(NoteEvent {
            id,
            section: self.sec,
            role,
            time: (time + j).max(0.0),
            dur,
            midi,
            gain,
            art,
        });
    }

    /// Section-local event in beats; cut if it starts past the section, and
    /// shortened to end with it.
    fn add(&mut self, role: Role, beat: f64, len: f64, midi: i32, gain: f64, art: Art) {
        if beat >= self.beats - 1e-9 || gain <= 0.0 {
            return;
        }
        let len = len.min(self.beats - beat);
        self.add_at(
            role,
            self.start + beat * self.spb,
            len * self.spb,
            midi,
            gain,
            art,
        );
    }

    fn vary(&mut self, amount: f64) -> f64 {
        1.0 + amount * (2.0 * self.rng.uniform() - 1.0)
    }
}

fn bar_len(beats: f64, bar: usize) -> f64 {
    (beats - (bar * 4) as f64).clamp(0.0, 4.0)
}

// ---- reusable figures -------------------------------------------------

const PICK_PATTERN: [usize; 8] = [0, 1, 2, 3, 2, 1, 2, 3];

/// The chip feel's levels, as shares of each role's nominal level.
mod chip {
    /// Synth-pluck: 0.28 on the downbeats, 0.2 off the beat.
    pub const PLUCK_OFF: f64 = 0.2 / 0.28;
    /// Soft bass 0.34 on the root, 0.24 on the octave (nominal 0.36).
    pub const BASS_ROOT: f64 = 0.34 / 0.36;
    pub const BASS_OCTAVE: f64 = 0.24 / 0.36;
    /// Shaker 0.16 off the beat and 0.12 on it (nominal 0.18).
    pub const SHAKER_OFF: f64 = 0.16 / 0.18;
    pub const SHAKER_ON: f64 = 0.12 / 0.18;
    /// Pad 0.12 (nominal 0.13).
    pub const PAD: f64 = 0.12 / 0.13;
}

/// Picking tones as played: like the guitar's, every arpeggio table is
/// written an octave below where it sounds.
fn picking_tones(_cx: &Ctx, chord: Chord) -> Vec<i32> {
    chord.guitar_tones()
}

fn picking_8ths(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    if w.cx.chip() {
        // Synth-pluck walks the 8-step arpeggio, an octave up like the guitar.
        let walk = chord.voicing().arpeggio;
        for e in 0..8 {
            let accent = if e % 2 == 0 { 1.0 } else { chip::PLUCK_OFF };
            let g = gain * accent * w.vary(0.05);
            w.add(
                Role::Picking,
                (bar * 4) as f64 + e as f64 * 0.5,
                0.5,
                walk[e % walk.len()] + 12,
                g,
                Art::Eighth,
            );
        }
        return;
    }
    let tones = chord.guitar_tones();
    for (e, &ix) in PICK_PATTERN.iter().enumerate() {
        let accent = if e % 2 == 0 { 1.0 } else { 0.75 };
        let g = gain * accent * w.vary(0.05);
        w.add(
            Role::Picking,
            (bar * 4) as f64 + e as f64 * 0.5,
            0.5,
            tones[ix],
            g,
            Art::Eighth,
        );
    }
}

fn picking_quarters(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    let tones = picking_tones(&w.cx, chord);
    for (q, &m) in tones.iter().enumerate() {
        let accent = if q == 0 { 1.0 } else { 0.8 };
        let g = gain * accent * w.vary(0.05);
        w.add(Role::Picking, (bar * 4 + q) as f64, 1.0, m, g, Art::Quarter);
    }
}

fn pad_bar(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    let mut gain = if w.cx.chip() { gain * chip::PAD } else { gain };
    // The pad tone is levelled for three-note chords; a 7th chord's four
    // voices share the same level.
    let voices = chord.voicing().pad.len();
    if voices != 3 {
        gain *= 3.0 / voices as f64;
    }
    w.pads.push((bar, chord, gain));
}

/// Writes the pad: each voice holds its note across consecutive bars while
/// the pitch and level stay the same, instead of re-attacking every bar.
fn flush_pads(w: &mut Writer) {
    let reqs = std::mem::take(&mut w.pads);
    let voices = reqs
        .iter()
        .map(|r| r.1.voicing().pad.len())
        .max()
        .unwrap_or(0);
    let pitch = |c: Chord, v: usize| c.voicing().pad.get(v).copied();
    for voice in 0..voices {
        let mut i = 0;
        while i < reqs.len() {
            let (bar, chord, gain) = reqs[i];
            let Some(m) = pitch(chord, voice) else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < reqs.len()
                && reqs[j].0 == reqs[j - 1].0 + 1
                && pitch(reqs[j].1, voice) == Some(m)
                && (reqs[j].2 - gain).abs() <= 0.1 * gain
            {
                j += 1;
            }
            let last = reqs[j - 1].0;
            let len = (last * 4) as f64 + bar_len(w.beats, last) - (bar * 4) as f64;
            w.add(Role::Pad, (bar * 4) as f64, len, m, gain, Art::Held);
            i = j;
        }
    }
}

fn bass_heavy(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    let root = w.cx.bass(chord);
    for (e, off) in [0, 0, 12, 0, 7, 0].iter().enumerate() {
        w.add(
            Role::Bass,
            (bar * 4) as f64 + e as f64 * 0.5,
            0.5,
            root + off,
            gain,
            Art::Normal,
        );
    }
    if bar % 2 == 1 {
        let off = *w.rng.pick(&[10, 5, 12]);
        w.add(
            Role::Bass,
            (bar * 4) as f64 + 3.5,
            0.5,
            root + off,
            gain * 0.9,
            Art::Normal,
        );
    }
}

fn bass_summer(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    let root = w.cx.bass(chord);
    for (pos, len, off) in [(0.0, 1.5, 0), (1.5, 0.5, 7), (2.0, 1.0, 0), (3.0, 1.0, 12)] {
        w.add(
            Role::Bass,
            (bar * 4) as f64 + pos,
            len,
            root + off,
            gain,
            Art::Normal,
        );
    }
}

/// Chip bass: 8ths alternating the root and its octave.
fn bass_chip(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    let root = w.cx.bass(chord);
    for e in 0..8 {
        let (off, g) = if e % 2 == 0 {
            (0, chip::BASS_ROOT)
        } else {
            (12, chip::BASS_OCTAVE)
        };
        w.add(
            Role::Bass,
            (bar * 4) as f64 + e as f64 * 0.5,
            0.5,
            root + off,
            gain * g,
            Art::Normal,
        );
    }
}

fn bass_groove(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    match w.cx.feel {
        Feel::Spring => bass_heavy(w, bar, chord, gain),
        Feel::Summer => bass_summer(w, bar, chord, gain),
        Feel::Chip => bass_chip(w, bar, chord, gain),
    }
}

fn bass_held(w: &mut Writer, bar: usize, chord: Chord, beats: f64, gain: f64) {
    let root = w.cx.bass(chord);
    let mut b = 0.0;
    while b < 4.0 {
        w.add(
            Role::Bass,
            (bar * 4) as f64 + b,
            beats,
            root,
            gain,
            Art::Held,
        );
        b += beats;
    }
}

fn kick_bar(w: &mut Writer, bar: usize, gain: f64) {
    let beats: &[usize] = match w.cx.feel {
        Feel::Spring | Feel::Chip => &[0, 2],
        Feel::Summer => &[0, 1, 2, 3],
    };
    for &b in beats {
        let g = if b == 0 { 1.0 } else { 0.8 };
        w.add(
            Role::Kick,
            (bar * 4 + b) as f64,
            0.5,
            36,
            gain * g,
            Art::Normal,
        );
    }
}

fn kick_1_3(w: &mut Writer, bar: usize, gain: f64) {
    w.add(Role::Kick, (bar * 4) as f64, 0.5, 36, gain, Art::Normal);
    w.add(
        Role::Kick,
        (bar * 4 + 2) as f64,
        0.5,
        36,
        gain * 0.8,
        Art::Normal,
    );
}

fn shaker_8ths(w: &mut Writer, bar: usize, gain: f64) {
    let (off, on) = if w.cx.chip() {
        (chip::SHAKER_OFF, chip::SHAKER_ON)
    } else {
        (1.0, 0.55)
    };
    for e in 0..8 {
        let g = if e % 2 == 1 { off } else { on };
        let v = w.vary(0.06);
        w.add(
            Role::Shaker,
            (bar * 4) as f64 + e as f64 * 0.5,
            0.25,
            0,
            gain * g * v,
            Art::Normal,
        );
    }
}

fn shaker_grid(w: &mut Writer, bar: usize, step: f64, gain: f64) {
    let n = (4.0 / step).round() as usize;
    for k in 0..n {
        let pos = k as f64 * step;
        let off = (pos * 2.0).round() as i64 % 2 == 1;
        let g = if off || step >= 1.0 { 1.0 } else { 0.55 };
        let v = w.vary(0.06);
        w.add(
            Role::Shaker,
            (bar * 4) as f64 + pos,
            step.min(0.25),
            0,
            gain * g * v,
            Art::Normal,
        );
    }
}

/// Rim (or the chip feel's clap) on 2 and 4.
fn backbeat_2_4(w: &mut Writer, bar: usize, gain: f64) {
    for b in [1, 3] {
        w.add(
            Role::Backbeat,
            (bar * 4 + b) as f64,
            0.25,
            0,
            gain,
            Art::Normal,
        );
    }
}

fn chord_hits(w: &mut Writer, bar: usize, chord: Chord, gain: f64) {
    for pos in [1.5, 3.5] {
        for &m in chord.voicing().pad {
            w.add(
                Role::ChordHits,
                (bar * 4) as f64 + pos,
                0.5,
                m + 12,
                gain,
                Art::Muted,
            );
        }
    }
}

fn melody_notes(
    w: &mut Writer,
    role: Role,
    notes: &[MelNote],
    offset_beats: f64,
    gain: f64,
    art: Art,
) {
    for n in notes {
        w.add(role, offset_beats + n.beat, n.len, n.midi, gain, art);
    }
}

// ---- section arrangers ------------------------------------------------

struct Shared {
    signature: Vec<MelNote>,
    /// Loop and melody the outro recaps.
    theme: (Loop, Vec<MelNote>),
}

/// `closing` is false when an event replaces the section's closing figure
/// (the build's run into the next section).
fn arrange_section(w: &mut Writer, sp: &SectionPlan, sh: &Shared, closing: bool) {
    let cx = w.cx;
    let pal = cx.pal;
    let nb = sp.bars;
    let ch = |b: usize| sp.chords[b];
    match sp.kind {
        VideoType::ColdOpenHook => {
            for b in 0..nb {
                picking_8ths(w, b, ch(b), 0.7);
                shaker_8ths(w, b, 0.85);
                kick_1_3(w, b, 0.75);
                let bass = cx.bass(ch(b));
                for pos in [0.0, 1.5, 2.0, 3.0] {
                    w.add(
                        Role::Bass,
                        (b * 4) as f64 + pos,
                        0.5,
                        bass,
                        0.6,
                        Art::Normal,
                    );
                }
            }
            // A 1-2 bar fragment of the signature motif on the first tonic
            // bar after the opening bar (or the opening bar if that is all).
            let bar = (1..nb).find(|&b| ch(b) == pal.tonic_chord).or(if nb <= 2 {
                Some(0)
            } else {
                None
            });
            if let Some(bar) = bar {
                let frag_bars = if nb - bar >= 3 { 2.0 } else { 1.0 };
                let frag: Vec<MelNote> = sh
                    .signature
                    .iter()
                    .filter(|n| n.beat < 4.0 * frag_bars)
                    .copied()
                    .collect();
                // Keep the fragment on chord tones under this section's chords.
                for n in frag {
                    let b = bar + (n.beat / 4.0) as usize;
                    if b >= nb {
                        break;
                    }
                    let pos = n.beat % 4.0;
                    let ok = !melody::is_strong(pos) || ch(b).is_chord_tone(n.midi);
                    if ok {
                        w.add(
                            Role::Motif,
                            (bar * 4) as f64 + n.beat,
                            n.len,
                            n.midi,
                            0.9,
                            Art::Normal,
                        );
                    }
                }
            }
        }
        VideoType::TitleSting => {
            for b in 0..nb {
                let c = ch(b);
                picking_8ths(w, b, c, 0.9);
                chord_hits(w, b, c, 1.0);
                pad_bar(w, b, c, 1.0);
                bass_groove(w, b, c, 1.0);
                kick_bar(w, b, 1.0);
                shaker_8ths(w, b, 1.0);
                backbeat_2_4(w, b, 1.0);
            }
            // The signature motif on lead and motif, repeated every 2 bars.
            let mut b = 0;
            while b < nb {
                melody_notes(
                    w,
                    Role::Lead,
                    &sh.signature,
                    (b * 4) as f64,
                    1.0,
                    Art::Normal,
                );
                melody_notes(
                    w,
                    Role::Motif,
                    &sh.signature,
                    (b * 4) as f64,
                    0.8,
                    Art::Normal,
                );
                b += 2;
            }
        }
        VideoType::Intro => {
            let mut lr = w.rng.fork("lead");
            let lo = cx.lead(74, 83).0;
            let long = melody::long_notes_in(&mut lr, &sp.chords, 2, lo);
            melody_notes(w, Role::Lead, &long, 0.0, 0.5, Art::Held);
            for b in 0..nb {
                picking_quarters(w, b, ch(b), 0.8);
                pad_bar(w, b, ch(b), 1.0);
                if b * 2 >= nb {
                    shaker_8ths(w, b, 0.75);
                }
            }
        }
        VideoType::Explain => {
            let mut mr = w.rng.fork("motif");
            for b in 0..nb {
                let c = ch(b);
                bass_groove(w, b, c, 0.8);
                kick_bar(w, b, 0.85);
                shaker_8ths(w, b, 0.8);
                backbeat_2_4(w, b, 0.55);
                // Speech section: picking low to keep 1-3 kHz clear.
                picking_8ths(w, b, c, 0.35);
                pad_bar(w, b, c, 0.75);
                if b % 2 == 1 {
                    let rep = melody::motif_reply_in(&mut mr, c, 2.0, pal.tonic, pal.offset);
                    melody_notes(w, Role::Motif, &rep, (b * 4) as f64, 0.6, Art::Normal);
                }
            }
        }
        VideoType::VisualDemo | VideoType::Reveal => {
            let reveal = sp.kind == VideoType::Reveal;
            let mel = section_melody(w.rng.fork("lead"), sp, &cx);
            melody_notes(
                w,
                Role::Lead,
                &mel,
                0.0,
                if reveal { 1.1 } else { 1.0 },
                Art::Normal,
            );
            for b in 0..nb {
                let c = ch(b);
                bass_groove(w, b, c, 1.0);
                kick_bar(w, b, 1.0);
                backbeat_2_4(w, b, if reveal { 1.3 } else { 1.0 });
                shaker_8ths(w, b, 1.0);
                picking_8ths(w, b, c, 1.0);
                chord_hits(w, b, c, 1.0);
                pad_bar(w, b, c, 0.9);
            }
        }
        VideoType::Build => {
            for b in 0..nb {
                let c = ch(b);
                let p = if nb > 1 {
                    b as f64 / (nb - 1) as f64
                } else {
                    1.0
                };
                if b * 2 < nb {
                    bass_held(w, b, c, 4.0, 0.8);
                } else {
                    let root = cx.bass(c);
                    for e in 0..8 {
                        let off = if e % 4 == 2 { 12 } else { 0 };
                        w.add(
                            Role::Bass,
                            (b * 4) as f64 + e as f64 * 0.5,
                            0.5,
                            root + off,
                            0.9,
                            Art::Normal,
                        );
                    }
                    kick_1_3(w, b, 0.9);
                    picking_8ths(w, b, c, 0.5 + 0.3 * p);
                }
                let step = if p < 1.0 / 3.0 {
                    1.0
                } else if p < 2.0 / 3.0 {
                    0.5
                } else {
                    0.25
                };
                shaker_grid(w, b, step, 0.55 + 0.55 * p);
                pad_bar(w, b, c, 0.7 + 0.5 * p);
            }
            // Mallet run up through the last two beats into the next section.
            if closing {
                let last = nb - 1;
                let start_beat = (last * 4) as f64 + (bar_len(w.beats, last) - 2.0).max(0.0);
                let c = ch(last);
                let off = pal.offset;
                let base = chord_tones(c, 74 + off, 79 + off)[0];
                let run: Vec<i32> = (base..=base + 14)
                    .filter(|&m| cx.in_scale(m))
                    .take(8)
                    .collect();
                for (k, &m) in run.iter().enumerate() {
                    let g = 0.6 + 0.5 * k as f64 / 7.0;
                    w.add(
                        Role::Motif,
                        start_beat + k as f64 * 0.25,
                        0.25,
                        m,
                        g,
                        Art::Normal,
                    );
                }
            }
        }
        VideoType::Reflect => {
            if nb >= 4 {
                let mut lr = w.rng.fork("lead");
                let lo = cx.lead(74, 83).0;
                let long = melody::long_notes_in(&mut lr, &sp.chords, 2, lo);
                melody_notes(w, Role::Lead, &long, 0.0, 0.45, Art::Held);
            }
            for b in 0..nb {
                pad_bar(w, b, ch(b), 1.0);
                picking_quarters(w, b, ch(b), 0.7);
                bass_held(w, b, ch(b), 4.0, 0.7);
            }
        }
        VideoType::Sponsor => {
            for b in 0..nb {
                pad_bar(w, b, ch(b), 0.9);
                bass_held(w, b, ch(b), 2.0, 0.85);
                picking_8ths(w, b, ch(b), 0.45);
                shaker_8ths(w, b, 0.65);
            }
        }
        VideoType::RecapCta => {
            let mut lr = w.rng.fork("lead");
            let (lo, hi) = (pal.mel_lo, pal.mel_hi);
            let (lo, hi) = lead_window(lo, hi, cx.shift);
            let mel = melody::generate_in(&mut lr, &sp.chords, lo, hi, true, pal.tonic);
            for n in &mel {
                let fall = 1.0 - 0.2 * n.beat / (nb * 4) as f64;
                w.add(Role::Lead, n.beat, n.len, n.midi, 0.55 * fall, Art::Normal);
            }
            for b in 0..nb {
                let c = ch(b);
                let fall = 1.0 - 0.2 * b as f64 / nb.max(1) as f64;
                picking_8ths(w, b, c, 0.6 * fall);
                pad_bar(w, b, c, 0.9 * fall);
                match cx.feel {
                    Feel::Summer => bass_summer(w, b, c, 0.95 * fall),
                    Feel::Spring => bass_held(w, b, c, 2.0, 0.95 * fall),
                    Feel::Chip => bass_chip(w, b, c, 0.95 * fall),
                }
                shaker_8ths(w, b, 0.75 * fall);
            }
        }
        VideoType::OutroEndScreen => {
            // Recap the theme over the same chords, then hold the final chord.
            let recap_bars = nb.saturating_sub(1).min(4);
            for n in sh
                .theme
                .1
                .iter()
                .filter(|n| n.beat < (recap_bars * 4) as f64)
            {
                w.add(Role::Lead, n.beat, n.len, n.midi, 0.8, Art::Normal);
            }
            if nb >= 2 {
                let (lo, hi) = lead_window(pal.mel_lo, pal.mel_hi, cx.shift);
                let prev = sh
                    .theme
                    .1
                    .iter()
                    .rfind(|n| n.beat < (recap_bars * 4) as f64)
                    .map_or(lo + 4, |n| n.midi);
                let d = chord_tones(pal.final_chord, lo, hi)
                    .into_iter()
                    .min_by_key(|m| (m - prev).abs())
                    .unwrap_or(lo);
                w.add(Role::Lead, ((nb - 1) * 4) as f64, 4.0, d, 0.7, Art::Held);
            }
            for b in 0..nb {
                let c = ch(b);
                if b + 1 < nb {
                    picking_quarters(w, b, c, 0.7);
                }
                bass_held(w, b, c, 4.0, 0.8);
                pad_bar(w, b, c, 0.6);
            }
            // Final strummed chord.
            let last = (nb - 1) * 4;
            for (k, m) in picking_tones(&cx, pal.final_chord).iter().enumerate() {
                let t = w.start + last as f64 * w.spb + k as f64 * 0.03;
                w.add_at(Role::Picking, t, 4.0, *m, 1.0, Art::Strum { ring: 4.0 });
            }
        }
    }
}

/// A visual-demo or reveal section's lead melody (also the outro's theme).
fn section_melody(mut lr: Rng, sp: &SectionPlan, cx: &Ctx) -> Vec<MelNote> {
    let pal = cx.pal;
    let hi = if sp.kind == VideoType::Reveal {
        pal.reveal_hi
    } else {
        pal.mel_hi
    };
    let (lo, hi) = lead_window(pal.mel_lo, hi, cx.shift);
    melody::generate_in(&mut lr, &sp.chords, lo, hi, false, pal.tonic)
}

/// What [`arrange_with`] needs besides the sections and their timing.
#[derive(Clone, Debug)]
pub struct ArrangeOptions {
    pub feel: Feel,
    pub seed: u64,
    /// Key shift in semitones from the feel's home key.
    pub shift: i32,
    /// Spell transposed chord names with flats.
    pub flats: bool,
    /// Boundaries (by index of the section they lead into) where an event
    /// replaces the automatic transition.
    pub suppressed: Vec<usize>,
}

/// Builds the whole arrangement.
pub fn arrange(kinds: &[VideoType], timing: &Timing, feel: Feel, seed: u64) -> Arrangement {
    arrange_with(
        kinds,
        timing,
        &ArrangeOptions {
            feel,
            seed,
            shift: 0,
            flats: false,
            suppressed: Vec::new(),
        },
    )
}

pub fn arrange_with(kinds: &[VideoType], timing: &Timing, opts: &ArrangeOptions) -> Arrangement {
    let feel = opts.feel;
    let root = Rng::new(opts.seed);
    let spb = timing.spb;
    let pal = palette(feel);
    let cx = Ctx {
        feel,
        pal,
        shift: opts.shift,
    };
    let flats = opts.flats;
    let name = |c: Chord| c.name_in(cx.shift, flats);

    // Chord loops: one choice per section, never repeating the previous
    // section's loop.
    let mut loop_rng = root.fork("loops");
    let mut loops: Vec<Loop> = Vec::new();
    let mut prev: Option<Loop> = None;
    for &k in kinds {
        let lp = match k {
            VideoType::Sponsor => pal.sponsor,
            VideoType::Reflect => pal.reflect,
            _ => pick_loop_from(&mut loop_rng, prev, &pal.general),
        };
        loops.push(lp);
        prev = Some(lp);
    }

    // Signature motif: two bars over the tonic and subdominant, the same
    // every time.
    let (sig_lo, sig_hi) = lead_window(pal.mel_lo, pal.mel_hi, cx.shift);
    let signature = melody::generate_in(
        &mut root.fork("signature"),
        &[pal.tonic_chord, pal.sub_chord],
        sig_lo,
        sig_hi,
        false,
        pal.tonic,
    );

    let mut sections: Vec<SectionPlan> = Vec::new();
    for (i, (&k, sn)) in kinds.iter().zip(&timing.sections).enumerate() {
        let beats = sn.beats();
        let bars = (beats as usize).div_ceil(4);
        let lp = loops[i];
        let (chords, loop_name) = match k {
            VideoType::TitleSting => (
                (0..bars)
                    .map(|b| {
                        if b % 2 == 0 {
                            pal.tonic_chord
                        } else {
                            pal.sub_chord
                        }
                    })
                    .collect(),
                format!(
                    "{} {} (signature)",
                    name(pal.tonic_chord),
                    name(pal.sub_chord)
                ),
            ),
            VideoType::RecapCta => (
                chords_ending_in(lp, bars, &pal.ending),
                format!(
                    "{} + {}",
                    lp.name_in(cx.shift, flats),
                    pal.ending.map(name).join(" ")
                ),
            ),
            _ => (chords_for(lp, bars), lp.name_in(cx.shift, flats)),
        };
        sections.push(SectionPlan {
            index: i,
            kind: k,
            start: sn.start,
            end: sn.end,
            beats,
            bars,
            chords,
            loop_name,
            preferred_bass: preferred_bass(feel, k),
            feel,
        });
    }

    // Theme for the outro recap: the first visual-demo/reveal melody, or a
    // fresh one if there is none.
    let theme = sections
        .iter()
        .find(|s| matches!(s.kind, VideoType::VisualDemo | VideoType::Reveal))
        .map(|s| {
            let lr = root
                .fork_n(1000 + s.index as u64)
                .fork("parts")
                .fork("lead");
            (loops[s.index], section_melody(lr, s, &cx))
        })
        .unwrap_or_else(|| {
            let lp = pal.general[0];
            let chords = chords_for(lp, 4);
            (
                lp,
                melody::generate_in(
                    &mut root.fork("theme"),
                    &chords,
                    sig_lo,
                    sig_hi,
                    false,
                    pal.tonic,
                ),
            )
        });
    // The outro plays over the theme's loop and ends on the final chord.
    for s in sections.iter_mut() {
        if s.kind == VideoType::OutroEndScreen {
            let mut c = chords_for(theme.0, s.bars);
            if let Some(last) = c.last_mut() {
                *last = pal.final_chord;
            }
            s.chords = c;
            s.loop_name = format!(
                "{} (theme) + {}",
                theme.0.name_in(cx.shift, flats),
                name(pal.final_chord)
            );
        }
    }

    let shared = Shared { signature, theme };
    let n = sections.len();
    let suppressed = |j: usize| opts.suppressed.contains(&j);

    let mut events: Vec<NoteEvent> = Vec::new();
    type State = ([u32; Role::COUNT], Rng);
    let mut writers_state: Vec<State> = Vec::new();
    for sp in &sections {
        let mut w = Writer::new(
            sp.index,
            sp.start,
            spb,
            sp.beats as f64,
            &root,
            cx,
            &mut events,
        );
        arrange_section(&mut w, sp, &shared, !suppressed(sp.index + 1));
        flush_pads(&mut w);
        writers_state.push((w.counters, w.hum.clone()));
    }

    // Transitions and hits, written by the owning section.
    let mut transitions = Vec::new();
    let hit_at = |owner: usize,
                  time: f64,
                  chord: Chord,
                  events: &mut Vec<NoteEvent>,
                  st: &mut Vec<State>| {
        let sp = &sections[owner];
        let mut w = Writer::new(owner, sp.start, spb, sp.beats as f64, &root, cx, events);
        w.counters = st[owner].0;
        w.hum = st[owner].1.clone();
        // Replace the owner's own kick on that downbeat.
        w.out.retain(|e| {
            !(e.section == owner && e.role == Role::Kick && (e.time - time).abs() < 0.02)
        });
        w.add_at(Role::Kick, time, 0.5 * spb, 36, 1.15, Art::Normal);
        for (k, m) in picking_tones(&cx, chord).iter().enumerate() {
            w.add_at(
                Role::Picking,
                time + k as f64 * 0.03,
                2.0 * spb,
                *m,
                1.0,
                Art::Strum { ring: 2.5 },
            );
        }
        w.add_at(Role::Bass, time, 2.0 * spb, cx.bass(chord), 1.0, Art::Held);
        st[owner] = (w.counters, w.hum.clone());
    };

    // A reveal (or title-sting... which has its own entry) at the very start.
    if sections[0].kind == VideoType::Reveal {
        hit_at(
            0,
            0.0,
            sections[0].chords[0],
            &mut events,
            &mut writers_state,
        );
    }

    let off = pal.offset;
    for j in 1..n {
        let (a, b) = (&sections[j - 1], &sections[j]);
        let t = b.start;
        let delta = b.kind.energy_span().0 - a.kind.energy_span().1;
        let owner = if a.kind == VideoType::TitleSting {
            j - 1
        } else {
            j
        };
        if suppressed(j) {
            transitions.push(TransitionInfo {
                boundary: j,
                time: t,
                from: a.kind,
                to: b.kind,
                energy_delta: delta,
                kind: "event",
                hit: false,
                owner,
            });
            continue;
        }
        let hit = matches!(b.kind, VideoType::TitleSting | VideoType::Reveal)
            || matches!(a.kind, VideoType::Build | VideoType::TitleSting);
        let kind = if delta >= 2.0 {
            "run-up"
        } else if delta >= 1.0 {
            "pickup"
        } else if delta <= -1.0 {
            "drop"
        } else {
            "none"
        };
        {
            let sp = &sections[j];
            let mut w = Writer::new(j, sp.start, spb, sp.beats as f64, &root, cx, &mut events);
            w.counters = writers_state[j].0;
            w.hum = writers_state[j].1.clone();
            let motif = 0.16; // nominal motif level, for absolute gains
            match kind {
                "run-up" => {
                    let run = [74, 76, 78, 81, 83, 86];
                    for (k, m) in run.iter().enumerate() {
                        let g = 0.10 + 0.10 * k as f64 / 5.0;
                        let time = t - 0.15 * (5 - k) as f64;
                        w.add_at(Role::Motif, time, 0.15, m + off, g / motif, Art::Normal);
                    }
                    // Shaker crescendo in 16ths over the last bar.
                    for k in 0..16 {
                        let time = t - spb * 4.0 + k as f64 * spb * 0.25;
                        if time >= a.start {
                            let g = 0.3 + 0.9 * k as f64 / 15.0;
                            w.add_at(Role::Shaker, time, 0.25 * spb, 0, g, Art::Normal);
                        }
                    }
                }
                "pickup" => {
                    let target = chord_tones(b.chords[0], 79 + off, 86 + off)[0];
                    let below: Vec<i32> =
                        (target - 7..target).filter(|&m| cx.in_scale(m)).collect();
                    let notes = &below[below.len() - 3..];
                    for (k, m) in notes.iter().enumerate() {
                        let time = t - spb * 0.5 * (3 - k) as f64;
                        w.add_at(
                            Role::Motif,
                            time,
                            0.5 * spb,
                            *m,
                            0.8 + 0.1 * k as f64,
                            Art::Normal,
                        );
                    }
                }
                "drop" => {
                    let last_chord = *a.chords.last().expect("bars");
                    let top = *chord_tones(last_chord, 80 + off, 88 + off)
                        .first()
                        .unwrap_or(&(83 + off));
                    let notes: Vec<i32> = (top - 6..=top)
                        .rev()
                        .filter(|&m| cx.in_scale(m))
                        .take(3)
                        .collect();
                    for (k, m) in notes.iter().enumerate() {
                        let time = t - spb * 0.5 * (3 - k) as f64;
                        w.add_at(Role::Motif, time, 0.5 * spb, *m, 0.8, Art::Normal);
                    }
                }
                _ => {}
            }
            writers_state[j] = (w.counters, w.hum.clone());
        }
        if kind == "drop" {
            // Drums rest for the last beat before the boundary.
            events.retain(|e| {
                !(e.section == j - 1
                    && matches!(e.role, Role::Kick | Role::Shaker | Role::Backbeat)
                    && e.time >= t - spb - 0.01
                    && e.time < t)
            });
        }
        if hit {
            let chord = sections[j].chords[0];
            hit_at(owner, t, chord, &mut events, &mut writers_state);
        }
        transitions.push(TransitionInfo {
            boundary: j,
            time: t,
            from: a.kind,
            to: b.kind,
            energy_delta: delta,
            kind,
            hit,
            owner,
        });
    }
    // A title-sting that ends the plan hits on its last downbeat instead.
    if sections[n - 1].kind == VideoType::TitleSting {
        let s = &sections[n - 1];
        let t = s.start + ((s.bars - 1) * 4) as f64 * spb;
        hit_at(n - 1, t, pal.tonic_chord, &mut events, &mut writers_state);
    }

    events.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.id.cmp(&b.id)));
    Arrangement {
        bpm: timing.bpm,
        spb,
        duration: timing.duration,
        feel,
        shift: cx.shift,
        sections,
        events,
        transitions,
    }
}
