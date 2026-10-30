//! Timed events: sounds tied to things happening on screen.
//!
//! - `run`: a staggered sequence of notes (a wave of pieces appearing).
//! - `ticks`: small repeated accents (steps, pops).
//! - `hit`: a one-shot accent.
//! - `riser`: a noise swell on its own.
//! - `entrance` / `exit`: a pixellab scene building up or breaking apart,
//!   scored piece by piece from its schedule.
//!
//! Event times are in video seconds. They snap to the music's 1/16 grid
//! when close (see [`Quant`]) and otherwise stay exactly where they are.
//! Every random choice comes from a stream named after the event's index,
//! so adding an event never changes anything else.

use crate::arrange::{Arrangement, Art};
use crate::harmony::{fold_bass, palette, Chord, MAJOR, PENTATONIC};
use crate::pixellab::{self, Scene};
use crate::plan::parse_time;
use crate::rng::Rng;
use crate::timing::{note_value_beats, Anchor, TICK_WEIGHT};
use crate::types::{Instrument, Role};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::Path;

/// A length given as a note value or in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dur {
    Beats(f64),
    Seconds(f64),
}

impl Dur {
    pub fn seconds(self, spb: f64) -> f64 {
        match self {
            Dur::Beats(b) => b * spb,
            Dur::Seconds(s) => s,
        }
    }
}

/// How event times meet the music's grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Quant {
    /// Every time stays exact.
    Off,
    /// Snap to this grid (in beats) when within [`SNAP_TOLERANCE`].
    Grid(f64),
}

/// Times within this of the grid snap to it; others stay exact and warn.
pub const SNAP_TOLERANCE: f64 = 0.015;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Contour {
    Up,
    Down,
    UpDown,
    DownUp,
    Wave,
    Random,
}

impl Contour {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "up" => Contour::Up,
            "down" => Contour::Down,
            "up-down" => Contour::UpDown,
            "down-up" => Contour::DownUp,
            "wave" => Contour::Wave,
            "random" => Contour::Random,
            _ => return None,
        })
    }

    /// Whether the line falls overall (its default gain ramp falls too).
    fn falls(self) -> bool {
        matches!(self, Contour::Down | Contour::DownUp)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleKind {
    Pentatonic,
    Major,
    /// Tones of the chord sounding at each note.
    Chord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RiserKind {
    None,
    Up,
    Reverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strength {
    Soft,
    Land,
    Pop,
    Big,
}

impl Strength {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "soft" => Strength::Soft,
            "land" => Strength::Land,
            "pop" => Strength::Pop,
            "big" => Strength::Big,
            _ => return None,
        })
    }

    fn level(self) -> f64 {
        match self {
            Strength::Soft => 0.6,
            Strength::Land | Strength::Pop => 1.0,
            Strength::Big => 1.2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunTiming {
    Step { count: usize, step: Dur },
    End { count: usize, end: f64 },
    Times(Vec<f64>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunSpec {
    pub start: f64,
    pub timing: RunTiming,
    pub direction: Contour,
    pub scale: ScaleKind,
    pub range: (i32, i32),
    pub gain: Option<(f64, f64)>,
    pub instrument: Option<Instrument>,
    pub riser: RiserKind,
    pub land: Option<f64>,
    pub quantize: Quant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TickTiming {
    Times(Vec<f64>),
    Every {
        start: f64,
        every: Vec<Dur>,
        count: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TickPitch {
    /// Seeded choice from the high pentatonic set.
    Seeded,
    Contour(Contour),
    Notes(Vec<i32>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicksSpec {
    pub timing: TickTiming,
    pub sound: Instrument,
    pub pitch: TickPitch,
    pub accent: Vec<f64>,
    pub gain: f64,
    pub quantize: Quant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HitSpec {
    pub at: f64,
    pub strength: Strength,
    pub quantize: Quant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiserSpec {
    pub start: f64,
    pub end: f64,
    pub reverse: bool,
    pub quantize: Quant,
}

/// When a piece's sound starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sync {
    Start,
    Impact,
    Settle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PitchMode {
    Time,
    X,
    Y,
    Order,
    Random,
}

/// An instrument for one kind of piece: the default role's choice, a
/// specific instrument, or silence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Voice {
    Default,
    Use(Instrument),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voices {
    pub ground: Voice,
    pub object: Voice,
    pub shadow: Voice,
    pub sky: Voice,
}

#[derive(Clone, Debug)]
pub struct SceneSpec {
    pub exit: bool,
    pub at: f64,
    pub scene: Scene,
    pub voices: Voices,
    pub sync: Option<Sync>,
    pub pitch: PitchMode,
    pub scale: ScaleKind,
    pub range: (i32, i32),
    /// Seconds.
    pub cluster: f64,
    pub max_rate: f64,
    /// The hit at the settle time (entrance `land`, exit `end`).
    pub land: Option<Strength>,
    pub riser: RiserKind,
    pub fps: Option<f64>,
    pub quantize: Quant,
}

#[derive(Clone, Debug)]
pub enum EventKind {
    Run(RunSpec),
    Ticks(TicksSpec),
    Hit(HitSpec),
    Riser(RiserSpec),
    Scene(Box<SceneSpec>),
}

#[derive(Clone, Debug)]
pub struct EventSpec {
    pub index: usize,
    pub kind: EventKind,
}

impl EventSpec {
    pub fn kind_id(&self) -> &'static str {
        match &self.kind {
            EventKind::Run(_) => "run",
            EventKind::Ticks(_) => "ticks",
            EventKind::Hit(_) => "hit",
            EventKind::Riser(_) => "riser",
            EventKind::Scene(s) if s.exit => "exit",
            EventKind::Scene(_) => "entrance",
        }
    }

    fn label(&self) -> String {
        format!("event {} ({})", self.index + 1, self.kind_id())
    }
}

// ------------------------------------------------------------------ parsing

/// What parsing events needs besides the JSON.
pub struct ParseContext<'a> {
    /// Directory `from` paths are relative to.
    pub base: &'a Path,
    /// `--pixellab-document`, for `scene` sources.
    pub document: Option<&'a Path>,
    pub seed: u64,
}

struct Obj<'a> {
    o: &'a Map<String, Value>,
    ctx: String,
}

impl Obj<'_> {
    fn has(&self, k: &str) -> bool {
        self.o.get(k).is_some_and(|v| !v.is_null())
    }

    fn get(&self, k: &str) -> Option<&Value> {
        self.o.get(k).filter(|v| !v.is_null())
    }

    fn time(&self, k: &str, errors: &mut Vec<String>) -> Option<f64> {
        let v = self.get(k)?;
        match parse_time_value(v) {
            Some(t) => Some(t),
            None => {
                errors.push(format!(
                    "{}: {k} must be seconds or \"m:ss.s\", not negative, got {v}",
                    self.ctx
                ));
                None
            }
        }
    }

    fn times(&self, k: &str, errors: &mut Vec<String>) -> Option<Vec<f64>> {
        let v = self.get(k)?;
        let Some(a) = v.as_array() else {
            errors.push(format!("{}: {k} must be a list of times", self.ctx));
            return None;
        };
        let mut out = Vec::new();
        for x in a {
            match parse_time_value(x) {
                Some(t) => out.push(t),
                None => errors.push(format!(
                    "{}: {k} holds {x}, which is not a time in seconds or \"m:ss.s\"",
                    self.ctx
                )),
            }
        }
        Some(out)
    }

    fn count(&self, k: &str, min: usize, errors: &mut Vec<String>) -> Option<usize> {
        let v = self.get(k)?;
        match v.as_u64() {
            Some(n) if n as usize >= min && n <= 4096 => Some(n as usize),
            _ => {
                errors.push(format!(
                    "{}: {k} must be a whole number from {min} to 4096, got {v}",
                    self.ctx
                ));
                None
            }
        }
    }

    fn number(&self, k: &str, lo: f64, hi: f64, errors: &mut Vec<String>) -> Option<f64> {
        let v = self.get(k)?;
        match v.as_f64() {
            Some(n) if n.is_finite() && (lo..=hi).contains(&n) => Some(n),
            _ => {
                errors.push(format!(
                    "{}: {k} must be a number from {lo} to {hi}, got {v}",
                    self.ctx
                ));
                None
            }
        }
    }

    fn string(&self, k: &str, errors: &mut Vec<String>) -> Option<&str> {
        let v = self.get(k)?;
        match v.as_str() {
            Some(s) => Some(s),
            None => {
                errors.push(format!("{}: {k} must be a string, got {v}", self.ctx));
                None
            }
        }
    }

    fn check_keys(&self, allowed: &[&str], errors: &mut Vec<String>) {
        for k in self.o.keys() {
            if !allowed.contains(&k.as_str()) {
                errors.push(format!(
                    "{}: unknown key \"{k}\" (expected {})",
                    self.ctx,
                    allowed.join(", ")
                ));
            }
        }
    }

    fn quantize(&self, default: Quant, errors: &mut Vec<String>) -> Quant {
        match self.get("quantize") {
            None => default,
            Some(Value::String(s)) if s == "off" => Quant::Off,
            Some(Value::String(s)) => match note_value_beats(s) {
                Some(b) => Quant::Grid(b),
                None => {
                    errors.push(format!(
                        "{}: quantize must be \"off\" or a note value like \"1/16\", got \"{s}\"",
                        self.ctx
                    ));
                    default
                }
            },
            Some(Value::Bool(false)) => Quant::Off,
            Some(v) => {
                errors.push(format!(
                    "{}: quantize must be \"off\" or a note value like \"1/16\", got {v}",
                    self.ctx
                ));
                default
            }
        }
    }

    fn dur(&self, v: &Value, what: &str, errors: &mut Vec<String>) -> Option<Dur> {
        match v {
            Value::String(s) => match note_value_beats(s) {
                Some(b) => Some(Dur::Beats(b)),
                None => {
                    errors.push(format!(
                        "{}: {what} must be a note value (\"1/8\", \"1/16\", \"1/32\", \"1/8t\") or seconds, got \"{s}\"",
                        self.ctx
                    ));
                    None
                }
            },
            Value::Number(n) => match n.as_f64() {
                Some(x) if x > 0.0 && x.is_finite() => Some(Dur::Seconds(x)),
                _ => {
                    errors.push(format!(
                        "{}: {what} must be more than 0 s, got {n}",
                        self.ctx
                    ));
                    None
                }
            },
            _ => {
                errors.push(format!(
                    "{}: {what} must be a note value or seconds, got {v}",
                    self.ctx
                ));
                None
            }
        }
    }

    fn range(&self, errors: &mut Vec<String>) -> (i32, i32) {
        let Some(v) = self.get("range") else {
            return (60, 88);
        };
        let ok = v.as_array().and_then(|a| match a.as_slice() {
            [lo, hi] => Some((lo.as_i64()?, hi.as_i64()?)),
            _ => None,
        });
        match ok {
            Some((lo, hi))
                if (0..=127).contains(&lo) && (0..=127).contains(&hi) && hi - lo >= 2 =>
            {
                (lo as i32, hi as i32)
            }
            _ => {
                errors.push(format!(
                    "{}: range must be [lowMidi, highMidi] within 0-127, at least 2 apart, got {v}",
                    self.ctx
                ));
                (60, 88)
            }
        }
    }

    fn scale(&self, errors: &mut Vec<String>) -> ScaleKind {
        match self.string("scale", errors) {
            None => ScaleKind::Pentatonic,
            Some("pentatonic") => ScaleKind::Pentatonic,
            Some("major") => ScaleKind::Major,
            Some("chord") => ScaleKind::Chord,
            Some(s) => {
                errors.push(format!(
                    "{}: scale must be pentatonic, major or chord, got \"{s}\"",
                    self.ctx
                ));
                ScaleKind::Pentatonic
            }
        }
    }

    fn riser(&self, default: RiserKind, errors: &mut Vec<String>) -> RiserKind {
        match self.get("riser") {
            None => default,
            Some(Value::Bool(true)) => RiserKind::Up,
            Some(Value::Bool(false)) => RiserKind::None,
            Some(Value::String(s)) if s == "reverse" => RiserKind::Reverse,
            Some(v) => {
                errors.push(format!(
                    "{}: riser must be true, false or \"reverse\", got {v}",
                    self.ctx
                ));
                default
            }
        }
    }

    fn instrument(&self, k: &str, errors: &mut Vec<String>) -> Option<Instrument> {
        let s = self.string(k, errors)?;
        match Instrument::from_id(s) {
            Some(i) => Some(i),
            None => {
                errors.push(format!(
                    "{}: unknown instrument \"{s}\" (valid: {})",
                    self.ctx,
                    instrument_ids()
                ));
                None
            }
        }
    }
}

fn instrument_ids() -> String {
    Instrument::ALL
        .iter()
        .map(|i| i.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Seconds, or a "m:ss.s" string.
pub fn parse_time_value(v: &Value) -> Option<f64> {
    let t = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => parse_time(s)?,
        _ => return None,
    };
    (t.is_finite() && t >= 0.0).then_some(t)
}

/// Parses the plan's `events` list. Every problem is reported.
pub fn parse_events(v: &Value, cx: &ParseContext, errors: &mut Vec<String>) -> Vec<EventSpec> {
    let Some(list) = v.as_array() else {
        errors.push("events must be a list of event objects".into());
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, e) in list.iter().enumerate() {
        let ctx = format!("event {}", i + 1);
        let Some(o) = e.as_object() else {
            errors.push(format!("{ctx}: must be an object with a \"kind\""));
            continue;
        };
        let kind = o.get("kind").and_then(Value::as_str).unwrap_or("");
        let ob = Obj {
            o,
            ctx: format!("{ctx} ({kind})"),
        };
        let before = errors.len();
        let parsed = match kind {
            "run" => parse_run(&ob, errors).map(EventKind::Run),
            "ticks" => parse_ticks(&ob, errors).map(EventKind::Ticks),
            "hit" => parse_hit(&ob, errors).map(EventKind::Hit),
            "riser" => parse_riser(&ob, errors).map(EventKind::Riser),
            "entrance" | "exit" => parse_scene(&ob, kind == "exit", i, cx, errors)
                .map(|s| EventKind::Scene(Box::new(s))),
            _ => {
                errors.push(format!(
                    "{ctx}: kind must be run, ticks, hit, riser, entrance or exit, got {}",
                    o.get("kind")
                        .map_or("nothing".to_string(), |k| k.to_string())
                ));
                None
            }
        };
        if let (Some(kind), true) = (parsed, errors.len() == before) {
            out.push(EventSpec { index: i, kind });
        }
    }
    out
}

fn parse_run(o: &Obj, errors: &mut Vec<String>) -> Option<RunSpec> {
    o.check_keys(
        &[
            "kind",
            "start",
            "count",
            "step",
            "end",
            "times",
            "direction",
            "scale",
            "range",
            "gain",
            "instrument",
            "riser",
            "land",
            "quantize",
        ],
        errors,
    );
    let start = o.time("start", errors);
    if !o.has("start") {
        errors.push(format!("{}: needs a start", o.ctx));
    }
    let land = o.time("land", errors);
    let (has_count, has_step, has_end, has_times) =
        (o.has("count"), o.has("step"), o.has("end"), o.has("times"));
    if has_times && (has_count || has_step || has_end) {
        errors.push(format!(
            "{}: give either times, or count with step or end, not both",
            o.ctx
        ));
    }
    if has_step && has_end {
        errors.push(format!(
            "{}: give step or end, not both (end sets the step)",
            o.ctx
        ));
    }
    let timing = if has_times {
        let t = o.times("times", errors)?;
        if t.is_empty() {
            errors.push(format!("{}: times is empty", o.ctx));
        }
        if t.windows(2).any(|w| w[1] < w[0]) {
            errors.push(format!("{}: times must not go backwards", o.ctx));
        }
        if let (Some(s), Some(&first)) = (start, t.first()) {
            if first < s {
                errors.push(format!("{}: times must not start before start", o.ctx));
            }
        }
        if let (Some(l), Some(&first)) = (land, t.first()) {
            if first >= l {
                errors.push(format!(
                    "{}: the first of times must come before land",
                    o.ctx
                ));
            }
        }
        RunTiming::Times(t)
    } else if has_step {
        if !has_count {
            errors.push(format!("{}: step needs a count", o.ctx));
        }
        let count = o.count("count", 1, errors)?;
        let step = o.dur(o.get("step")?, "step", errors)?;
        RunTiming::Step { count, step }
    } else if has_end {
        if !has_count {
            errors.push(format!("{}: end needs a count", o.ctx));
        }
        let count = o.count("count", 2, errors)?;
        let end = o.time("end", errors)?;
        if start.is_some_and(|s| end <= s) {
            errors.push(format!("{}: end must be after start", o.ctx));
        }
        RunTiming::End { count, end }
    } else {
        errors.push(format!(
            "{}: needs count with step or end, or a list of times",
            o.ctx
        ));
        return None;
    };
    let direction = match o.string("direction", errors) {
        None => Contour::Up,
        Some(s) => match Contour::parse(s).filter(|c| *c != Contour::Wave) {
            Some(c) => c,
            None => {
                errors.push(format!(
                    "{}: direction must be up, down, up-down, down-up or random, got \"{s}\"",
                    o.ctx
                ));
                Contour::Up
            }
        },
    };
    let gain = o.get("gain").and_then(|v| {
        let pair = v.as_array().and_then(|a| match a.as_slice() {
            [a, b] => Some((a.as_f64()?, b.as_f64()?)),
            _ => None,
        });
        match pair {
            Some((a, b)) if (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b) => Some((a, b)),
            _ => {
                errors.push(format!(
                    "{}: gain must be [from, to], each from 0 to 1, got {v}",
                    o.ctx
                ));
                None
            }
        }
    });
    if let (Some(s), Some(l)) = (start, land) {
        if l <= s {
            errors.push(format!("{}: land must be after start", o.ctx));
        }
    }
    Some(RunSpec {
        start: start?,
        timing,
        direction,
        scale: o.scale(errors),
        range: o.range(errors),
        gain,
        instrument: o.instrument("instrument", errors),
        riser: o.riser(RiserKind::None, errors),
        land,
        quantize: o.quantize(Quant::Grid(0.25), errors),
    })
}

fn parse_ticks(o: &Obj, errors: &mut Vec<String>) -> Option<TicksSpec> {
    o.check_keys(
        &[
            "kind", "times", "start", "every", "count", "sound", "contour", "notes", "accent",
            "gain", "quantize",
        ],
        errors,
    );
    let timing = if o.has("times") {
        if o.has("start") || o.has("every") || o.has("count") {
            errors.push(format!(
                "{}: give either times, or start with every and count, not both",
                o.ctx
            ));
        }
        TickTiming::Times(o.times("times", errors)?)
    } else if o.has("every") {
        if !o.has("start") || !o.has("count") {
            errors.push(format!("{}: every needs a start and a count", o.ctx));
        }
        let start = o.time("start", errors)?;
        let count = o.count("count", 1, errors)?;
        let ev = o.get("every")?;
        let every: Vec<Dur> = match ev {
            Value::Array(a) => {
                if a.is_empty() {
                    errors.push(format!("{}: every is empty", o.ctx));
                }
                a.iter().filter_map(|x| o.dur(x, "every", errors)).collect()
            }
            x => o.dur(x, "every", errors).into_iter().collect(),
        };
        TickTiming::Every {
            start,
            every,
            count,
        }
    } else {
        errors.push(format!(
            "{}: needs times, or start with every and count",
            o.ctx
        ));
        return None;
    };
    let sound = match o.string("sound", errors) {
        None | Some("blip") => Instrument::Blip,
        Some("mallet") => Instrument::Mallet,
        Some("clap") => Instrument::Clap,
        Some("shaker") => Instrument::Shaker,
        Some(s) => {
            errors.push(format!(
                "{}: sound must be blip, mallet, clap or shaker, got \"{s}\"",
                o.ctx
            ));
            Instrument::Blip
        }
    };
    if o.has("contour") && o.has("notes") {
        errors.push(format!("{}: give contour or notes, not both", o.ctx));
    }
    let pitch = if let Some(s) = o.string("contour", errors) {
        match Contour::parse(s) {
            Some(c) if matches!(c, Contour::Up | Contour::Down | Contour::Wave) => {
                TickPitch::Contour(c)
            }
            _ => {
                errors.push(format!(
                    "{}: contour must be up, down or wave, got \"{s}\"",
                    o.ctx
                ));
                TickPitch::Seeded
            }
        }
    } else if let Some(v) = o.get("notes") {
        let notes: Option<Vec<i32>> = v.as_array().and_then(|a| {
            a.iter()
                .map(|x| {
                    x.as_i64()
                        .filter(|m| (0..=127).contains(m))
                        .map(|m| m as i32)
                })
                .collect()
        });
        match notes {
            Some(n) if !n.is_empty() => TickPitch::Notes(n),
            _ => {
                errors.push(format!("{}: notes must be a list of MIDI numbers", o.ctx));
                TickPitch::Seeded
            }
        }
    } else {
        TickPitch::Seeded
    };
    Some(TicksSpec {
        timing,
        sound,
        pitch,
        accent: o.times("accent", errors).unwrap_or_default(),
        gain: o.number("gain", 0.0, 4.0, errors).unwrap_or(1.0),
        quantize: o.quantize(Quant::Grid(0.25), errors),
    })
}

fn parse_strength(o: &Obj, k: &str, default: Strength, errors: &mut Vec<String>) -> Strength {
    match o.string(k, errors) {
        None => default,
        Some(s) => Strength::parse(s).unwrap_or_else(|| {
            errors.push(format!(
                "{}: {k} must be soft, land, pop or big, got \"{s}\"",
                o.ctx
            ));
            default
        }),
    }
}

fn parse_hit(o: &Obj, errors: &mut Vec<String>) -> Option<HitSpec> {
    o.check_keys(&["kind", "at", "strength", "quantize"], errors);
    if !o.has("at") {
        errors.push(format!("{}: needs at", o.ctx));
    }
    Some(HitSpec {
        at: o.time("at", errors)?,
        strength: parse_strength(o, "strength", Strength::Land, errors),
        quantize: o.quantize(Quant::Grid(0.25), errors),
    })
}

fn parse_riser(o: &Obj, errors: &mut Vec<String>) -> Option<RiserSpec> {
    o.check_keys(&["kind", "start", "end", "reverse", "quantize"], errors);
    if !o.has("start") || !o.has("end") {
        errors.push(format!("{}: needs start and end", o.ctx));
    }
    let start = o.time("start", errors)?;
    let end = o.time("end", errors)?;
    if end <= start {
        errors.push(format!("{}: end must be after start", o.ctx));
    }
    let reverse = match o.get("reverse") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(v) => {
            errors.push(format!("{}: reverse must be true or false, got {v}", o.ctx));
            false
        }
    };
    Some(RiserSpec {
        start,
        end,
        reverse,
        quantize: o.quantize(Quant::Grid(0.25), errors),
    })
}

fn parse_voice(o: &Obj, v: &Value, k: &str, errors: &mut Vec<String>) -> Voice {
    match v.as_str() {
        Some("none") => Voice::None,
        Some(s) => match Instrument::from_id(s) {
            Some(i) => Voice::Use(i),
            None => {
                errors.push(format!(
                    "{}: voices.{k}: unknown instrument \"{s}\" (valid: {}, or none)",
                    o.ctx,
                    instrument_ids()
                ));
                Voice::Default
            }
        },
        None => {
            errors.push(format!(
                "{}: voices.{k} must be an instrument id or \"none\"",
                o.ctx
            ));
            Voice::Default
        }
    }
}

fn parse_scene(
    o: &Obj,
    exit: bool,
    index: usize,
    cx: &ParseContext,
    errors: &mut Vec<String>,
) -> Option<SceneSpec> {
    o.check_keys(
        &[
            "kind",
            "from",
            "pieces",
            "timing",
            "siteTiming",
            "grid",
            "scene",
            "at",
            "voices",
            "sync",
            "pitch",
            "scale",
            "range",
            "cluster",
            "maxRate",
            if exit { "end" } else { "land" },
            "riser",
            "fps",
            "quantize",
        ],
        errors,
    );
    let sources: Vec<&str> = ["from", "pieces", "grid", "scene"]
        .into_iter()
        .filter(|k| o.has(k))
        .collect();
    if sources.len() != 1 {
        errors.push(format!(
            "{}: give exactly one source: from, pieces (+ timing), grid (+ timing) or scene{}",
            o.ctx,
            if sources.is_empty() {
                String::new()
            } else {
                format!(" (got {})", sources.join(", "))
            }
        ));
        return None;
    }
    if o.has("timing") && o.has("siteTiming") {
        errors.push(format!(
            "{}: give timing (ms) or siteTiming (seconds), not both",
            o.ctx
        ));
    }
    if matches!(sources[0], "from" | "scene") && (o.has("timing") || o.has("siteTiming")) {
        errors.push(format!(
            "{}: a {} source carries its own timing; leave out timing and siteTiming",
            o.ctx, sources[0]
        ));
    }
    let timing = || -> Result<pixellab::Timing, Vec<String>> {
        if let Some(t) = o.get("siteTiming") {
            pixellab::timing_from_site(t)
        } else {
            pixellab::timing_from_entrance(o.get("timing").unwrap_or(&Value::Object(Map::new())))
        }
    };
    let ctx = o.ctx.clone();
    let prefix = |e: Vec<String>| {
        e.into_iter()
            .map(|m| format!("{ctx}: {m}"))
            .collect::<Vec<_>>()
    };
    let scene = match sources[0] {
        "from" => {
            let path = o.string("from", errors)?;
            let full = cx.base.join(path);
            let text = match std::fs::read_to_string(&full) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(format!("{}: cannot read {}: {e}", o.ctx, full.display()));
                    return None;
                }
            };
            let v: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}: {} is not JSON: {e}", o.ctx, full.display()));
                    return None;
                }
            };
            match pixellab::scene_from_show(&v, &format!("{} ({path})", o.ctx)) {
                Ok(s) => s,
                Err(e) => {
                    errors.extend(e);
                    return None;
                }
            }
        }
        "scene" => {
            let name = o.string("scene", errors)?;
            let Some(doc) = cx.document else {
                errors.push(format!(
                    "{}: \"scene\": \"{name}\" needs --pixellab-document <file.pixellab>",
                    o.ctx
                ));
                return None;
            };
            let v = match pixellab::show_with_pixellab(name, doc) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}: {e}", o.ctx));
                    return None;
                }
            };
            match pixellab::scene_from_show(&v, &o.ctx) {
                Ok(mut s) => {
                    s.source = "scene";
                    s
                }
                Err(e) => {
                    errors.extend(e);
                    return None;
                }
            }
        }
        "pieces" => {
            let pieces = pixellab::read_pieces(o.get("pieces")?, &o.ctx, errors);
            let t = match timing() {
                Ok(t) => t,
                Err(e) => {
                    errors.extend(prefix(e));
                    return None;
                }
            };
            if pieces.is_empty() {
                errors.push(format!("{}: pieces is empty", o.ctx));
                return None;
            }
            Scene::from_pieces(pieces, t, "pieces")
        }
        _ => {
            let g = o.get("grid")?;
            let Some(go) = g.as_object() else {
                errors.push(format!(
                    "{}: grid must be {{ cols, rows, objects, shadow, sky }}",
                    o.ctx
                ));
                return None;
            };
            let gob = Obj {
                o: go,
                ctx: format!("{} grid", o.ctx),
            };
            gob.check_keys(&["cols", "rows", "objects", "shadow", "sky"], errors);
            let flag = |k: &str, d: bool, errors: &mut Vec<String>| match go.get(k) {
                None => d,
                Some(Value::Bool(b)) => *b,
                Some(v) => {
                    errors.push(format!(
                        "{} grid: {k} must be true or false, got {v}",
                        o.ctx
                    ));
                    d
                }
            };
            let cols = gob.count("cols", 1, errors).unwrap_or(1);
            let rows = gob.count("rows", 1, errors).unwrap_or(1);
            if !gob.has("cols") || !gob.has("rows") {
                errors.push(format!("{} grid: needs cols and rows", o.ctx));
            }
            let objects = gob.count("objects", 0, errors).unwrap_or(0);
            let shadow = flag("shadow", true, errors);
            let sky = flag("sky", false, errors);
            let t = match timing() {
                Ok(t) => t,
                Err(e) => {
                    errors.extend(prefix(e));
                    return None;
                }
            };
            let mut rng = Rng::new(cx.seed).fork(&format!("events/{index}/grid"));
            let pieces = pixellab::grid_pieces(cols, rows, objects, shadow, sky, &mut rng);
            let mut s = Scene::from_pieces(pieces, t, "grid");
            s.approximate = true;
            s
        }
    };
    let at = o.time("at", errors).unwrap_or(0.0);
    let mut voices = Voices {
        ground: Voice::Default,
        object: Voice::Default,
        shadow: Voice::Default,
        sky: Voice::Default,
    };
    if let Some(v) = o.get("voices") {
        match v.as_object() {
            Some(m) => {
                for (k, x) in m {
                    let slot = match k.as_str() {
                        "ground" => &mut voices.ground,
                        "object" => &mut voices.object,
                        "shadow" => &mut voices.shadow,
                        "sky" => &mut voices.sky,
                        _ => {
                            errors.push(format!(
                                "{}: voices: unknown key \"{k}\" (expected ground, object, shadow, sky)",
                                o.ctx
                            ));
                            continue;
                        }
                    };
                    *slot = parse_voice(o, x, k, errors);
                }
            }
            None => errors.push(format!(
                "{}: voices must be {{ ground, object, shadow, sky }}",
                o.ctx
            )),
        }
    }
    let sync = match o.string("sync", errors) {
        None => None,
        Some("start") => Some(Sync::Start),
        Some("impact") => Some(Sync::Impact),
        Some("settle") => Some(Sync::Settle),
        Some(s) => {
            errors.push(format!(
                "{}: sync must be start, impact or settle, got \"{s}\"",
                o.ctx
            ));
            None
        }
    };
    let pitch = match o.string("pitch", errors) {
        None | Some("time") => PitchMode::Time,
        Some("x") => PitchMode::X,
        Some("y") => PitchMode::Y,
        Some("order") => PitchMode::Order,
        Some("random") => PitchMode::Random,
        Some(s) => {
            errors.push(format!(
                "{}: pitch must be time, x, y, order or random, got \"{s}\"",
                o.ctx
            ));
            PitchMode::Time
        }
    };
    let land_key = if exit { "end" } else { "land" };
    let land = match o.get(land_key) {
        None => Some(if exit { Strength::Pop } else { Strength::Land }),
        Some(Value::Bool(false)) => None,
        Some(_) => Some(parse_strength(o, land_key, Strength::Land, errors)),
    };
    let fps = o.number("fps", 1.0, 1000.0, errors);
    Some(SceneSpec {
        exit,
        at,
        scene,
        voices,
        sync,
        pitch,
        scale: o.scale(errors),
        range: o.range(errors),
        cluster: o.number("cluster", 0.0, 1000.0, errors).unwrap_or(35.0) / 1000.0,
        max_rate: o.number("maxRate", 0.1, 200.0, errors).unwrap_or(14.0),
        land,
        riser: o.riser(
            if exit {
                RiserKind::Reverse
            } else {
                RiserKind::Up
            },
            errors,
        ),
        fps,
        quantize: o.quantize(Quant::Off, errors),
    })
}

// ------------------------------------------------------------------ timing

/// Times the tempo search should fit (ticks count most).
pub fn anchors(events: &[EventSpec]) -> Vec<Anchor> {
    let mut out = Vec::new();
    let one = |time: f64| Anchor { time, weight: 1.0 };
    for e in events {
        match &e.kind {
            EventKind::Run(r) => {
                out.push(one(r.start));
                if let Some(l) = r.land {
                    out.push(one(l));
                }
                match &r.timing {
                    RunTiming::End { end, .. } => out.push(one(*end)),
                    RunTiming::Times(t) => out.extend(t.iter().map(|&x| one(x))),
                    RunTiming::Step { .. } => {}
                }
            }
            EventKind::Ticks(t) => {
                if let TickTiming::Times(ts) = &t.timing {
                    out.extend(ts.iter().map(|&time| Anchor {
                        time,
                        weight: TICK_WEIGHT,
                    }));
                } else if let TickTiming::Every { start, .. } = &t.timing {
                    out.push(Anchor {
                        time: *start,
                        weight: TICK_WEIGHT,
                    });
                }
            }
            EventKind::Hit(h) => out.push(one(h.at)),
            EventKind::Riser(r) => {
                out.push(one(r.start));
                out.push(one(r.end));
            }
            EventKind::Scene(s) => {
                out.push(one(s.at));
                out.push(one(s.at + s.scene.settled_by));
            }
        }
    }
    out
}

fn snap(t: f64, q: Quant, spb: f64) -> (f64, bool) {
    match q {
        Quant::Off => (t, true),
        Quant::Grid(beats) => {
            let g = beats * spb;
            let n = (t / g).round();
            let d = t - n * g;
            if d.abs() <= SNAP_TOLERANCE {
                (n * g, true)
            } else {
                (t, false)
            }
        }
    }
}

/// Counts the times that stayed exact, for one warning per event.
struct Snapper {
    q: Quant,
    spb: f64,
    total: usize,
    missed: usize,
}

impl Snapper {
    fn new(q: Quant, spb: f64) -> Self {
        Snapper {
            q,
            spb,
            total: 0,
            missed: 0,
        }
    }

    fn at(&mut self, t: f64) -> f64 {
        let (x, ok) = snap(t, self.q, self.spb);
        if self.q != Quant::Off {
            self.total += 1;
            if !ok {
                self.missed += 1;
            }
        }
        x
    }

    fn warning(&self, label: &str) -> Option<String> {
        let Quant::Grid(b) = self.q else { return None };
        (self.missed > 0).then(|| {
            format!(
                "{label}: {} of {} times are more than {:.0} ms off the {} grid and stay exact",
                self.missed,
                self.total,
                SNAP_TOLERANCE * 1000.0,
                grid_name(b)
            )
        })
    }
}

fn grid_name(beats: f64) -> String {
    let d = 4.0 / beats;
    if (d - d.round()).abs() < 1e-9 {
        format!("1/{}", d.round())
    } else {
        format!("{beats}-beat")
    }
}

/// When a run ends (its last note plus a step) and lands, for deciding
/// which boundaries keep their automatic transition.
pub fn run_end_times(events: &[EventSpec], spb: f64) -> Vec<f64> {
    let mut out = Vec::new();
    for e in events {
        match &e.kind {
            EventKind::Run(r) => {
                let times = run_times(r, spb).0;
                if let (Some(&last), Some(step)) = (times.last(), run_step(r, &times, spb)) {
                    out.push(last + step);
                }
                if let Some(l) = r.land {
                    out.push(snap(l, r.quantize, spb).0);
                }
            }
            EventKind::Scene(s) => out.push(s.at + s.scene.settled_by),
            _ => {}
        }
    }
    out
}

/// A run's note times (snapped start, compressed to land before `land`),
/// and the warning when it had to be compressed.
fn run_times(r: &RunSpec, spb: f64) -> (Vec<f64>, Option<(f64, f64)>) {
    let start = snap(r.start, r.quantize, spb).0;
    let mut times: Vec<f64> = match &r.timing {
        RunTiming::Step { count, step } => {
            let s = step.seconds(spb);
            (0..*count).map(|k| start + k as f64 * s).collect()
        }
        RunTiming::End { count, end } => {
            let end = snap(*end, r.quantize, spb).0;
            let s = (end - start) / (*count - 1) as f64;
            (0..*count).map(|k| start + k as f64 * s).collect()
        }
        RunTiming::Times(t) => t.iter().map(|&x| snap(x, r.quantize, spb).0).collect(),
    };
    let mut compressed = None;
    if let Some(land) = r.land {
        let land = snap(land, r.quantize, spb).0;
        let last = *times.last().unwrap_or(&start);
        if last >= land - 1e-9 && !times.is_empty() {
            let n = times.len() as f64;
            let first = times[0];
            let old = run_step(r, &times, spb).unwrap_or(0.0);
            if times.len() == 1 {
                times[0] = first.min(land - 0.05).max(0.0);
            } else {
                // Keep the shape; the landing becomes the next step.
                let span = last - first;
                let k = if span > 0.0 {
                    (land - first) * (n - 1.0) / (n * span)
                } else {
                    0.0
                };
                for t in times.iter_mut() {
                    *t = first + (*t - first) * k;
                }
            }
            let new = (land - first) / n;
            compressed = Some((old, new));
        }
    }
    (times, compressed)
}

fn run_step(r: &RunSpec, times: &[f64], spb: f64) -> Option<f64> {
    match &r.timing {
        RunTiming::Step { step, .. } if times.len() < 2 => Some(step.seconds(spb)),
        _ if times.len() >= 2 => {
            Some((times[times.len() - 1] - times[0]) / (times.len() - 1) as f64)
        }
        _ => None,
    }
}

// ------------------------------------------------------------------ notes

/// How loud an event note is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// An absolute amplitude, whatever instrument plays it.
    Abs(f64),
    /// A share of the resolved instrument's nominal level for the role.
    Rel(f64),
}

/// A note an event plays, before instruments are bound.
#[derive(Clone, Debug, PartialEq)]
pub struct EventNote {
    pub event: usize,
    /// What it is, for the cue sheet ("run", "tick", "accent", "kick", ...).
    pub what: &'static str,
    pub role: Role,
    /// The instrument asked for; the role's fallbacks apply otherwise.
    pub prefer: Option<Instrument>,
    pub time: f64,
    pub dur: f64,
    /// Absolute MIDI number (already in the score's key).
    pub midi: i32,
    pub level: Level,
    pub art: Art,
    /// The time before frame alignment, when `fps` moved it.
    pub sync_time: Option<f64>,
    /// The scene piece (index into its pieces) this sound is for.
    pub piece: Option<usize>,
}

/// What realising one event worked out, for the cue sheet.
#[derive(Clone, Debug, Serialize)]
pub struct EventSummary {
    pub index: usize,
    pub kind: &'static str,
    /// Times as resolved (seconds): start/end/land/at, depending on kind.
    pub resolved: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneSummary>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneSummary {
    pub source: &'static str,
    pub name: Option<String>,
    pub approximate: bool,
    pub settled_by_s: f64,
    pub settled_by_ms: f64,
    pub pieces: usize,
    pub sync_points: usize,
    pub clusters: usize,
    pub notes_kept: usize,
    pub fps: Option<f64>,
}

/// Everything the events play.
#[derive(Clone, Debug, Default)]
pub struct Realized {
    pub notes: Vec<EventNote>,
    pub summaries: Vec<EventSummary>,
    pub warnings: Vec<String>,
}

/// Context for turning events into notes.
pub struct RealizeContext<'a> {
    pub arr: &'a Arrangement,
    pub seed: u64,
    /// `--fps`, for entrance and exit events without their own.
    pub fps: Option<f64>,
}

impl RealizeContext<'_> {
    fn tonic(&self) -> i32 {
        (palette(self.arr.feel).tonic + self.arr.shift).rem_euclid(12)
    }

    /// The target key's tonic relative to C, between -6 and +5, for note
    /// sets written in C.
    fn c_offset(&self) -> i32 {
        let t = self.tonic();
        if t <= 5 {
            t
        } else {
            t - 12
        }
    }

    fn chord_at(&self, t: f64) -> Chord {
        self.arr.chord_at(t)
    }

    /// A chord's tones in the score's key.
    fn chord_pcs(&self, chord: Chord) -> Vec<i32> {
        chord
            .pitch_classes()
            .iter()
            .map(|p| (p + self.arr.shift).rem_euclid(12))
            .collect()
    }

    fn pool(&self, scale: ScaleKind, range: (i32, i32), t: f64) -> Vec<i32> {
        let tonic = self.tonic();
        let (lo, hi) = range;
        match scale {
            ScaleKind::Pentatonic => (lo..=hi)
                .filter(|m| PENTATONIC.contains(&(m - tonic).rem_euclid(12)))
                .collect(),
            ScaleKind::Major => (lo..=hi)
                .filter(|m| MAJOR.contains(&(m - tonic).rem_euclid(12)))
                .collect(),
            ScaleKind::Chord => {
                let pcs = self.chord_pcs(self.chord_at(t));
                (lo..=hi)
                    .filter(|m| pcs.contains(&m.rem_euclid(12)))
                    .collect()
            }
        }
    }

    fn rng(&self, index: usize, what: &str) -> Rng {
        Rng::new(self.seed).fork(&format!("events/{index}/{what}"))
    }
}

/// Index into a pool of `p` notes for note `k` of `n` along a contour.
pub fn contour_index(
    c: Contour,
    k: usize,
    n: usize,
    p: usize,
    rng: &mut Rng,
    prev: Option<usize>,
) -> usize {
    if p <= 1 {
        return 0;
    }
    let top = p - 1;
    let spread = |pos: usize, max: usize| -> usize {
        // Positions 0..=max, top-aligned when they fit, else stretched.
        if max <= top {
            top - max + pos
        } else {
            ((pos as f64) * top as f64 / max as f64).round() as usize
        }
    };
    match c {
        Contour::Up => spread(k, n.saturating_sub(1)),
        Contour::Down => spread(n.saturating_sub(1) - k, n.saturating_sub(1)),
        Contour::UpDown => {
            let peak = n / 2;
            spread(peak - peak.abs_diff(k).min(peak), peak)
        }
        Contour::DownUp => {
            let peak = n / 2;
            spread(peak.abs_diff(k).min(peak), peak)
        }
        Contour::Wave => {
            // A triangle wave over up to 5 notes, centred in the pool.
            let w = top.min(4);
            let period = 2 * w;
            let ph = k % period.max(1);
            let pos = if ph <= w { ph } else { period - ph };
            (top - w) / 2 + pos
        }
        Contour::Random => loop {
            let i = rng.below(p);
            if Some(i) != prev || p == 1 {
                break i;
            }
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn push(
    out: &mut Vec<EventNote>,
    event: usize,
    what: &'static str,
    role: Role,
    prefer: Option<Instrument>,
    time: f64,
    dur: f64,
    midi: i32,
    level: Level,
    art: Art,
) {
    out.push(EventNote {
        event,
        what,
        role,
        prefer,
        time,
        dur,
        midi,
        level,
        art,
        sync_time: None,
        piece: None,
    });
}

/// A hit at `at`: a kick, the chord an octave up (motif role) and a bass
/// root, plus the sparkle (`land`), the pop (`pop`) or both (`big`).
fn hit_notes(cx: &RealizeContext, event: usize, at: f64, s: Strength, out: &mut Vec<EventNote>) {
    let k = s.level();
    let spb = cx.arr.spb;
    let chord = cx.chord_at(at);
    let shift = cx.arr.shift;
    push(
        out,
        event,
        "hit kick",
        Role::Kick,
        None,
        at,
        0.5 * spb,
        36,
        Level::Rel(1.1 * k),
        Art::Normal,
    );
    for &m in chord.voicing().pad {
        push(
            out,
            event,
            "hit chord",
            Role::Motif,
            Some(Instrument::Mallet),
            at,
            2.0 * spb,
            m + 12 + shift,
            Level::Abs(0.07 * k),
            Art::Normal,
        );
    }
    let bass = fold_bass(chord.voicing().bass + shift);
    push(
        out,
        event,
        "hit bass",
        Role::Bass,
        None,
        at,
        2.0 * spb,
        bass,
        Level::Rel(0.9 * k),
        Art::Held,
    );
    let c = cx.c_offset();
    if matches!(s, Strength::Land | Strength::Big) {
        for (j, m) in [96, 100, 103, 108].iter().enumerate() {
            push(
                out,
                event,
                "sparkle",
                Role::Fx,
                Some(Instrument::Blip),
                at + 0.03 * j as f64,
                0.14,
                m + c,
                Level::Abs(0.11 - 0.015 * j as f64),
                Art::Normal,
            );
        }
    }
    if matches!(s, Strength::Pop | Strength::Big) {
        push(
            out,
            event,
            "pop blip",
            Role::Fx,
            Some(Instrument::Blip),
            at,
            0.14,
            84 + c,
            Level::Abs(0.2 * k),
            Art::Normal,
        );
        push(
            out,
            event,
            "pop chip",
            Role::Motif,
            Some(Instrument::Chip),
            at,
            0.12,
            72 + c,
            Level::Abs(0.12 * k),
            Art::Muted,
        );
    }
}

fn riser_note(
    event: usize,
    start: f64,
    end: f64,
    reverse: bool,
    prefer: Option<Instrument>,
    out: &mut Vec<EventNote>,
) {
    if end - start < 0.02 {
        return;
    }
    push(
        out,
        event,
        if reverse { "reverse riser" } else { "riser" },
        Role::Riser,
        prefer,
        start,
        end - start,
        0,
        Level::Rel(1.0),
        if reverse { Art::Reverse } else { Art::Normal },
    );
}

fn secs(map: &mut Map<String, Value>, k: &str, t: f64) {
    map.insert(k.into(), pixellab::num((t * 1e6).round() / 1e6));
}

/// Turns every event into notes.
pub fn realize(events: &[EventSpec], cx: &RealizeContext) -> Realized {
    let mut r = Realized::default();
    for e in events {
        let mut out = Vec::new();
        let mut summary = EventSummary {
            index: e.index,
            kind: e.kind_id(),
            resolved: Map::new(),
            scene: None,
            warnings: Vec::new(),
        };
        let label = e.label();
        match &e.kind {
            EventKind::Run(run) => realize_run(cx, e.index, run, &label, &mut out, &mut summary),
            EventKind::Ticks(t) => realize_ticks(cx, e.index, t, &label, &mut out, &mut summary),
            EventKind::Hit(h) => {
                let mut sn = Snapper::new(h.quantize, cx.arr.spb);
                let at = sn.at(h.at);
                secs(&mut summary.resolved, "at", at);
                hit_notes(cx, e.index, at, h.strength, &mut out);
                summary.warnings.extend(sn.warning(&label));
            }
            EventKind::Riser(rs) => {
                let mut sn = Snapper::new(rs.quantize, cx.arr.spb);
                let (a, b) = (sn.at(rs.start), sn.at(rs.end));
                secs(&mut summary.resolved, "start", a);
                secs(&mut summary.resolved, "end", b);
                riser_note(e.index, a, b, rs.reverse, None, &mut out);
                summary.warnings.extend(sn.warning(&label));
            }
            EventKind::Scene(s) => realize_scene(cx, e.index, s, &label, &mut out, &mut summary),
        }
        r.warnings.extend(summary.warnings.iter().cloned());
        r.notes.extend(out);
        r.summaries.push(summary);
    }
    r
}

fn realize_run(
    cx: &RealizeContext,
    index: usize,
    run: &RunSpec,
    label: &str,
    out: &mut Vec<EventNote>,
    summary: &mut EventSummary,
) {
    let spb = cx.arr.spb;
    let mut sn = Snapper::new(run.quantize, spb);
    let start = sn.at(run.start);
    if let RunTiming::End { end, .. } = &run.timing {
        sn.at(*end);
    }
    if let RunTiming::Times(t) = &run.timing {
        for &x in t {
            sn.at(x);
        }
    }
    let land = run.land.map(|l| sn.at(l));
    let (times, compressed) = run_times(run, spb);
    if let Some((old, new)) = compressed {
        summary.warnings.push(format!(
            "{label}: its last note would fall at or after its landing at {:.3} s; step compressed from {old:.4} s to {new:.4} s",
            land.unwrap_or(start)
        ));
    }
    let n = times.len();
    let step = run_step(run, &times, spb).unwrap_or(0.1);
    let (g0, g1) = run.gain.unwrap_or(if run.direction.falls() {
        (0.2, 0.09)
    } else {
        (0.09, 0.2)
    });
    let mut rng = cx.rng(index, "run");
    let mut prev = None;
    for (k, &t) in times.iter().enumerate() {
        let pool = cx.pool(run.scale, run.range, t);
        if pool.is_empty() {
            continue;
        }
        let i = contour_index(run.direction, k, n, pool.len(), &mut rng, prev);
        prev = Some(i);
        let g = if n > 1 {
            g0 + (g1 - g0) * k as f64 / (n - 1) as f64
        } else {
            g0
        };
        push(
            out,
            index,
            "run",
            Role::Motif,
            run.instrument,
            t,
            step.max(0.05),
            pool[i],
            Level::Abs(g),
            Art::Normal,
        );
    }
    let end = times.last().map_or(start, |l| l + step);
    secs(&mut summary.resolved, "start", start);
    secs(&mut summary.resolved, "step", step);
    secs(
        &mut summary.resolved,
        "last_note",
        *times.last().unwrap_or(&start),
    );
    secs(&mut summary.resolved, "end", end);
    match run.riser {
        RiserKind::Up => riser_note(index, start, land.unwrap_or(end), false, None, out),
        RiserKind::Reverse => riser_note(index, start, end, true, None, out),
        RiserKind::None => {}
    }
    if let Some(l) = land {
        secs(&mut summary.resolved, "land", l);
        hit_notes(cx, index, l, Strength::Land, out);
    }
    summary.warnings.extend(sn.warning(label));
}

fn tick_times(t: &TicksSpec, sn: &mut Snapper) -> Vec<f64> {
    match &t.timing {
        TickTiming::Times(ts) => ts.iter().map(|&x| sn.at(x)).collect(),
        TickTiming::Every {
            start,
            every,
            count,
        } => {
            let mut at = sn.at(*start);
            let mut out = Vec::with_capacity(*count);
            for k in 0..*count {
                out.push(at);
                if !every.is_empty() {
                    at += every[k % every.len()].seconds(sn.spb);
                }
            }
            out
        }
    }
}

fn realize_ticks(
    cx: &RealizeContext,
    index: usize,
    t: &TicksSpec,
    label: &str,
    out: &mut Vec<EventNote>,
    summary: &mut EventSummary,
) {
    let mut sn = Snapper::new(t.quantize, cx.arr.spb);
    let times = tick_times(t, &mut sn);
    let accents: Vec<f64> = t.accent.iter().map(|&a| sn.at(a)).collect();
    let set: Vec<i32> = [84, 86, 88, 91, 93, 96]
        .iter()
        .map(|m| m + cx.c_offset())
        .collect();
    let mut rng = cx.rng(index, "ticks");
    let mut prev: Option<usize> = None;
    let n = times.len();
    let mut matched = vec![false; accents.len()];
    for (k, &time) in times.iter().enumerate() {
        let midi = match &t.pitch {
            TickPitch::Notes(ns) => ns[k % ns.len()],
            TickPitch::Contour(c) => set[contour_index(*c, k, n, set.len(), &mut rng, prev)],
            TickPitch::Seeded => {
                let i = contour_index(Contour::Random, k, n, set.len(), &mut rng, prev);
                prev = Some(i);
                set[i]
            }
        };
        let accent = accents.iter().position(|&a| (a - time).abs() <= 0.02);
        if let Some(a) = accent {
            matched[a] = true;
        }
        let g = t.gain * if accent.is_some() { 1.5 } else { 1.0 };
        // Clap and shaker ignore the pitch, but a pitched fallback uses it.
        push(
            out,
            index,
            if accent.is_some() { "accent" } else { "tick" },
            Role::Fx,
            Some(t.sound),
            time,
            0.14,
            midi,
            Level::Rel(g),
            Art::Normal,
        );
        if accent.is_some() {
            let m2 = midi + 5;
            push(
                out,
                index,
                "accent echo",
                Role::Fx,
                Some(t.sound),
                time + 0.05,
                0.14,
                m2,
                Level::Rel(t.gain),
                Art::Normal,
            );
        }
    }
    for (a, ok) in accents.iter().zip(&matched) {
        if !ok {
            summary.warnings.push(format!(
                "{label}: accent at {a:.3} s matches no tick (within 20 ms)"
            ));
        }
    }
    summary.resolved.insert(
        "times".into(),
        Value::Array(
            times
                .iter()
                .map(|&x| pixellab::num((x * 1e6).round() / 1e6))
                .collect(),
        ),
    );
    summary.warnings.extend(sn.warning(label));
}

/// One ground or object piece's sync point before merging.
#[derive(Clone, Copy, Debug)]
struct Point {
    piece: usize,
    /// Video time.
    time: f64,
    /// 0-1, decides the pitch.
    p: f64,
    object: bool,
    /// Checker group (0 or 1) on a checker ground.
    group: Option<u8>,
}

fn realize_scene(
    cx: &RealizeContext,
    index: usize,
    s: &SceneSpec,
    label: &str,
    out: &mut Vec<EventNote>,
    summary: &mut EventSummary,
) {
    let sc = &s.scene;
    let settled = sc.settled_by;
    let mirror = |local: f64| if s.exit { settled - local } else { local };
    let video = |local: f64| s.at + mirror(local);
    let places = pixellab::places(&sc.pieces);
    let mut prng = cx.rng(index, "pitch");

    // Pitch keys for every ground and object piece.
    let sync_local = |e: &pixellab::Entrance| -> f64 {
        let default = match e.mv.as_str() {
            "drop" | "pop" => Sync::Impact,
            _ => Sync::Start,
        };
        match s.sync.unwrap_or(default) {
            Sync::Start => e.delay,
            Sync::Impact => e.delay + e.peak * e.duration,
            Sync::Settle => e.delay + e.duration,
        }
    };
    // Pieces voiced "none" make no sound and take no share of the rate.
    let silent = |object: bool| {
        if object {
            s.voices.object == Voice::None
        } else {
            s.voices.ground == Voice::None
        }
    };
    let mut raw: Vec<(usize, f64)> = Vec::new(); // (piece, local sync time)
    let mut keys_x = Vec::new();
    let mut keys_y = Vec::new();
    let mut idx = Vec::new();
    for (i, (p, e)) in sc.pieces.iter().zip(&sc.entrances).enumerate() {
        let Some(e) = e else { continue };
        if !matches!(p.enter.as_str(), "ground" | "object") || silent(p.enter == "object") {
            continue;
        }
        idx.push(i);
        let pl = places[i].unwrap_or([0.0; 4]);
        keys_x.push(pl[2]);
        keys_y.push(pl[3]);
        raw.push((i, sync_local(e)));
    }
    let locals: Vec<f64> = raw.iter().map(|r| r.1).collect();
    let keys: Vec<f64> = match s.pitch {
        PitchMode::Time => pixellab::normalize(&locals),
        PitchMode::X => pixellab::normalize(&keys_x),
        PitchMode::Y => pixellab::normalize(&keys_y),
        PitchMode::Order => idx.iter().map(|&i| sc.order[i].unwrap_or(0.0)).collect(),
        PitchMode::Random => idx.iter().map(|_| prng.uniform()).collect(),
    };
    let checker = sc.timing.ground == "checker";
    let mut points: Vec<Point> = raw
        .iter()
        .zip(&keys)
        .map(|(&(piece, local), &p)| {
            let object = sc.pieces[piece].enter == "object";
            Point {
                piece,
                time: video(local),
                p,
                object,
                group: (checker && !object).then(|| sc.order[piece].unwrap_or(0.0).round() as u8),
            }
        })
        .collect();
    let sync_points = points.len();
    points.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.piece.cmp(&b.piece)));

    // Merge points within `cluster` of a cluster's first point, per kind.
    struct Cluster {
        first: Point,
        count: usize,
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    for object in [false, true] {
        let mut cur: Option<Cluster> = None;
        for pt in points.iter().filter(|p| p.object == object) {
            match &mut cur {
                Some(c) if pt.time - c.first.time <= s.cluster + 1e-12 => c.count += 1,
                _ => {
                    if let Some(c) = cur.take() {
                        clusters.push(c);
                    }
                    cur = Some(Cluster {
                        first: *pt,
                        count: 1,
                    });
                }
            }
        }
        clusters.extend(cur);
    }
    clusters.sort_by(|a, b| {
        a.first
            .time
            .total_cmp(&b.first.time)
            .then(a.first.piece.cmp(&b.first.piece))
    });
    let n_clusters = clusters.len();
    // At most `max_rate` notes a second: keep a note only once 1/max_rate
    // has passed since the last one kept.
    let gap = 1.0 / s.max_rate;
    let mut kept: Vec<&Cluster> = Vec::new();
    for c in &clusters {
        if kept
            .last()
            .is_none_or(|k| c.first.time - k.first.time >= gap - 1e-9)
        {
            kept.push(c);
        }
    }

    let voice = |v: Voice| -> Option<Option<Instrument>> {
        match v {
            Voice::Default => Some(None),
            Voice::Use(i) => Some(Some(i)),
            Voice::None => None,
        }
    };
    let together = |object: bool| {
        let t = &sc.timing;
        if object {
            !t.with && (t.objects == "together" || (t.objects == "same" && t.ground == "together"))
        } else {
            t.ground == "together"
        }
    };
    let shift = cx.arr.shift;
    let mut notes_kept = 0;
    for c in &kept {
        let pt = c.first;
        let from = out.len();
        let Some(e) = &sc.entrances[pt.piece] else {
            continue;
        };
        let (role, prefer, base) = if pt.object {
            match voice(s.voices.object) {
                Some(p) => (Role::Fx, p, 0.13),
                None => continue,
            }
        } else {
            match voice(s.voices.ground) {
                Some(p) => (Role::Motif, p, 0.11),
                None => continue,
            }
        };
        let gain = (base * (1.0 + 0.25 * libm::log(c.count as f64))).min(1.6 * base);
        let t = pt.time;
        let pool = cx.pool(s.scale, s.range, t);
        if pool.is_empty() {
            continue;
        }
        let one_cluster = clusters
            .iter()
            .filter(|k| k.first.object == pt.object)
            .count()
            == 1;
        if together(pt.object) && one_cluster && c.count > 1 {
            // Every piece at one time: a single chord.
            let chord = cx.chord_at(t);
            for &m in chord.voicing().pad {
                push(
                    out,
                    index,
                    "piece chord",
                    role,
                    prefer,
                    t,
                    0.4,
                    m + 12 + shift,
                    Level::Abs(base * 0.8),
                    Art::Normal,
                );
            }
            notes_kept += 1;
            out[from..]
                .iter_mut()
                .for_each(|n| n.piece = Some(pt.piece));
            continue;
        }
        let midi = match pt.group {
            Some(g) => {
                // Checker: the two groups alternate between root and fifth.
                let pcs = cx.chord_pcs(cx.chord_at(t));
                let mid = (s.range.0 + s.range.1) / 2;
                let near = |pc: i32, from: i32| {
                    (from..from + 12)
                        .find(|m| m.rem_euclid(12) == pc)
                        .unwrap_or(from)
                };
                let a = near(pcs[0], mid - 6);
                let b = near(pcs[pcs.len().min(3) - 1], a + 1);
                if g == 0 {
                    a
                } else {
                    b
                }
            }
            None => pool[((pt.p.clamp(0.0, 1.0)) * (pool.len() - 1) as f64).round() as usize],
        };
        let what = if pt.object {
            "piece object"
        } else {
            "piece ground"
        };
        notes_kept += 1;
        match e.mv.as_str() {
            "float" => push(
                out,
                index,
                what,
                role,
                prefer,
                t,
                0.3,
                midi,
                Level::Abs(gain * 0.7),
                Art::Glide,
            ),
            "drop" => {
                push(
                    out,
                    index,
                    what,
                    role,
                    prefer,
                    t,
                    0.3,
                    midi,
                    Level::Abs(gain),
                    Art::Normal,
                );
                push(
                    out,
                    index,
                    "thud",
                    Role::Kick,
                    None,
                    t,
                    0.2,
                    36,
                    Level::Abs(0.15),
                    Art::Normal,
                );
            }
            "pop" => push(
                out,
                index,
                what,
                role,
                prefer,
                t,
                0.2,
                midi,
                Level::Abs(gain),
                Art::Muted,
            ),
            "stepped" => {
                push(
                    out,
                    index,
                    what,
                    role,
                    prefer,
                    t,
                    0.15,
                    midi,
                    Level::Abs(gain),
                    Art::Muted,
                );
                let repeats = (e.rise.floor().max(0.0) as usize).min(3);
                for j in 1..=repeats {
                    let local = e.delay + e.duration * j as f64 / (repeats + 1) as f64;
                    let g = gain * libm::pow(0.5, j as f64);
                    push(
                        out,
                        index,
                        "step",
                        role,
                        prefer,
                        video(local),
                        0.1,
                        midi,
                        Level::Abs(g),
                        Art::Muted,
                    );
                }
            }
            _ => push(
                out,
                index,
                what,
                role,
                prefer,
                t,
                0.3,
                midi,
                Level::Abs(gain),
                Art::Swell,
            ),
        }
        out[from..]
            .iter_mut()
            .for_each(|n| n.piece = Some(pt.piece));
    }

    // Shadow and sky.
    let c = cx.c_offset();
    for (p, e) in sc.pieces.iter().zip(&sc.entrances) {
        let Some(e) = e else { continue };
        let (a, b) = if s.exit {
            let (x, y) = pixellab::exit_window(e, settled);
            (s.at + x, s.at + y)
        } else {
            (s.at + e.delay, s.at + e.delay + e.duration)
        };
        match p.enter.as_str() {
            "shadow" => {
                if let Some(prefer) = voice(s.voices.shadow) {
                    riser_note(index, a, b, s.exit, prefer, out);
                }
            }
            "sky" => {
                if let Some(prefer) = voice(s.voices.sky) {
                    let chord = cx.chord_at(a);
                    for &m in chord.voicing().pad {
                        push(
                            out,
                            index,
                            "sky swell",
                            Role::Pad,
                            prefer,
                            a,
                            b - a,
                            m + shift,
                            Level::Rel(0.7),
                            Art::Held,
                        );
                    }
                    // Where the drift ends (on an exit, where it begins).
                    let at = if s.exit { a } else { b };
                    for (j, m) in [96, 103].iter().enumerate() {
                        push(
                            out,
                            index,
                            "sky sparkle",
                            Role::Fx,
                            Some(Instrument::Blip),
                            at + 0.06 * j as f64,
                            0.14,
                            m + c,
                            Level::Abs(0.08),
                            Art::Normal,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // The swell across the entrance (or exit) and the hit when it settles.
    let end = s.at + settled;
    let first = kept.first().map_or(s.at, |k| k.first.time);
    match s.riser {
        RiserKind::Up => riser_note(index, first, end, false, None, out),
        RiserKind::Reverse => riser_note(index, first, end, true, None, out),
        RiserKind::None => {}
    }
    if let Some(strength) = s.land {
        hit_notes(cx, index, end, strength, out);
    }

    // Quantize (off by default) and align to frames.
    let mut sn = Snapper::new(s.quantize, cx.arr.spb);
    let fps = s.fps.or(cx.fps);
    for n in out.iter_mut() {
        let q = sn.at(n.time);
        let f = match fps {
            Some(fps) => (q * fps - 1e-6).ceil() / fps,
            None => q,
        };
        if (f - n.time).abs() > 1e-12 {
            n.sync_time = Some(n.time);
        }
        n.time = f;
    }
    secs(&mut summary.resolved, "at", s.at);
    secs(
        &mut summary.resolved,
        if s.exit { "end" } else { "settled" },
        end,
    );
    summary.scene = Some(SceneSummary {
        source: sc.source,
        name: sc.name.clone(),
        approximate: sc.approximate,
        settled_by_s: settled,
        settled_by_ms: (settled * 1000.0 * 1e6).round() / 1e6,
        pieces: sc.pieces.len(),
        sync_points,
        clusters: n_clusters,
        notes_kept,
        fps,
    });
    summary.warnings.extend(sn.warning(label));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contours_order_the_pool() {
        let mut r = Rng::new(1);
        let up: Vec<usize> = (0..5)
            .map(|k| contour_index(Contour::Up, k, 5, 8, &mut r, None))
            .collect();
        assert_eq!(up, vec![3, 4, 5, 6, 7]);
        let down: Vec<usize> = (0..5)
            .map(|k| contour_index(Contour::Down, k, 5, 8, &mut r, None))
            .collect();
        assert_eq!(down, vec![7, 6, 5, 4, 3]);
        let ud: Vec<usize> = (0..5)
            .map(|k| contour_index(Contour::UpDown, k, 5, 8, &mut r, None))
            .collect();
        assert_eq!(ud, vec![5, 6, 7, 6, 5]);
        let du: Vec<usize> = (0..5)
            .map(|k| contour_index(Contour::DownUp, k, 5, 8, &mut r, None))
            .collect();
        assert_eq!(du, vec![7, 6, 5, 6, 7]);
        let wave: Vec<usize> = (0..10)
            .map(|k| contour_index(Contour::Wave, k, 10, 8, &mut r, None))
            .collect();
        assert_eq!(wave, vec![1, 2, 3, 4, 5, 4, 3, 2, 1, 2]);
        // More notes than the pool: stretched, never out of range.
        let up: Vec<usize> = (0..12)
            .map(|k| contour_index(Contour::Up, k, 12, 4, &mut r, None))
            .collect();
        assert!(up.windows(2).all(|w| w[1] >= w[0]) && up[0] == 0 && up[11] == 3);
    }
}
