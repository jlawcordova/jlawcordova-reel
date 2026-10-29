//! pixellab entrances, exits and sequences, ported from pixellab's engine
//! (`crates/engine/src/entrance.rs` and `sequence.rs` in
//! github.com/jlawcordova/jlawcordova-lab at commit 8a3179f) function for
//! function, so the same pieces and timing give the same schedule. Tested
//! against `fixtures/pixellab/parity.json`, pixellab's own parity fixture.
//!
//! The schedule works in seconds, as pixellab's does; an entrance
//! configuration keeps its times in whole ms, as pixellab stores it.

use crate::rng::Rng;
use serde_json::{Map, Value};
use std::path::Path;

/// Half of pixellab's 44x22 tile, in scene pixels: a column steps
/// (HALF_W, HALF_H), a row (-HALF_W, HALF_H).
pub const HALF_W: f64 = 22.0;
pub const HALF_H: f64 = 11.0;

/// The orders the ground can enter in. Each is a score; lower enters first.
pub const GROUND_ORDERS: [&str; 11] = [
    "rows",
    "cols",
    "back-front",
    "front-back",
    "sweep",
    "center",
    "edges",
    "clock",
    "checker",
    "random",
    "together",
];
/// The orders the objects can enter in, after the ground. `same` follows
/// the ground's order.
pub const OBJECT_ORDERS: [&str; 6] = [
    "ltr",
    "same",
    "back-front",
    "front-back",
    "random",
    "together",
];
/// How each ground or object piece moves in.
pub const MOVES: [&str; 5] = ["float", "drop", "pop", "stepped", "fade"];
/// How the objects move in: `same` as the ground, or a move of their own.
pub const OBJECT_MOVES: [&str; 6] = ["same", "float", "drop", "pop", "stepped", "fade"];
/// The roles a piece can have.
pub const ROLES: [&str; 4] = ["shadow", "ground", "object", "sky"];

/// What kind of value a field of `entrance` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Whole milliseconds, 0 to 60000 (seconds in `Timing`).
    Ms,
    /// A share of a move, 0 to 1.
    Ratio,
    /// A size, 0 to 4.
    Scale,
    /// Art pixels.
    Px,
    GroundOrder,
    ObjectOrder,
    Move,
    ObjectMove,
    Flag,
    Seed,
}

/// A field's default value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Default {
    Num(f64),
    Str(&'static str),
    Bool(bool),
}

impl Default {
    pub fn to_value(self) -> Value {
        match self {
            Default::Num(n) => num(n),
            Default::Str(s) => Value::String(s.to_string()),
            Default::Bool(b) => Value::Bool(b),
        }
    }
}

/// Every field of an entrance, in pixellab's order, with pixellab's default.
/// Times are in ms.
pub const FIELDS: [(&str, Kind, Default); 28] = [
    ("start", Kind::Ms, Default::Num(200.0)),
    ("groundDelay", Kind::Ms, Default::Num(120.0)),
    ("groundSpan", Kind::Ms, Default::Num(1000.0)),
    ("cascade", Kind::Ms, Default::Num(250.0)),
    ("ground", Kind::GroundOrder, Default::Str("sweep")),
    ("objects", Kind::ObjectOrder, Default::Str("same")),
    ("with", Kind::Flag, Default::Bool(true)),
    ("objectLead", Kind::Ms, Default::Num(80.0)),
    ("objectSpan", Kind::Ms, Default::Num(600.0)),
    ("lag", Kind::Ms, Default::Num(500.0)),
    ("move", Kind::Move, Default::Str("float")),
    ("objectMove", Kind::ObjectMove, Default::Str("drop")),
    ("duration", Kind::Ms, Default::Num(350.0)),
    ("peak", Kind::Ratio, Default::Num(0.6)),
    ("rise", Kind::Px, Default::Num(16.0)),
    ("overshoot", Kind::Px, Default::Num(6.0)),
    ("popFrom", Kind::Scale, Default::Num(0.3)),
    ("popPeak", Kind::Scale, Default::Num(1.1)),
    ("objectDuration", Kind::Ms, Default::Num(300.0)),
    ("objectPeak", Kind::Ratio, Default::Num(0.5)),
    ("objectRise", Kind::Px, Default::Num(24.0)),
    ("objectOvershoot", Kind::Px, Default::Num(1.5)),
    ("objectPopFrom", Kind::Scale, Default::Num(0.3)),
    ("objectPopPeak", Kind::Scale, Default::Num(1.1)),
    ("shadowDuration", Kind::Ms, Default::Num(400.0)),
    ("skyDuration", Kind::Ms, Default::Num(600.0)),
    ("skyDrift", Kind::Px, Default::Num(6.0)),
    ("seed", Kind::Seed, Default::Num(1.0)),
];

// ------------------------------------------------------------------ JavaScript numbers

/// JavaScript's `Math.round`: halves round up.
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

fn is_integer(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0
}

/// A JSON number, written as an integer when it is one.
pub fn num(n: f64) -> Value {
    if is_integer(n) && n.abs() < 9.007_199_254_740_992e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn qs(s: &str) -> String {
    format!("'{s}'")
}

// ------------------------------------------------------------------ the configuration

/// The problems with an entrance configuration, each as pixellab words it.
pub fn validate(entrance: &Value, add: &mut dyn FnMut(String)) {
    let Some(e) = entrance.as_object() else {
        return add("entrance must be an object of timing fields (or left out)".into());
    };
    for key in e.keys() {
        let Some((_, kind, _)) = FIELDS.iter().find(|(k, _, _)| k == key) else {
            add(format!("entrance: unknown property {}", qs(key)));
            continue;
        };
        let v = &e[key];
        let n = v.as_f64();
        let one_of = |list: &[&str]| v.as_str().is_some_and(|s| list.contains(&s));
        let problem = match kind {
            Kind::Ms => (!n.is_some_and(|n| is_integer(n) && (0.0..=60000.0).contains(&n)))
                .then(|| format!("entrance.{key} must be a whole number of ms from 0 to 60000")),
            Kind::Ratio => (!n.is_some_and(|n| (0.0..=1.0).contains(&n)))
                .then(|| format!("entrance.{key} must be a number from 0 to 1")),
            Kind::Scale => (!n.is_some_and(|n| (0.0..=4.0).contains(&n)))
                .then(|| format!("entrance.{key} must be a scale from 0 to 4")),
            Kind::Px => (!n.is_some_and(|n| n.is_finite() && n.abs() <= 4096.0))
                .then(|| format!("entrance.{key} must be a number of pixels from -4096 to 4096")),
            Kind::GroundOrder => (!one_of(&GROUND_ORDERS))
                .then(|| format!("entrance.{key} must be one of {}", GROUND_ORDERS.join(", "))),
            Kind::ObjectOrder => (!one_of(&OBJECT_ORDERS))
                .then(|| format!("entrance.{key} must be one of {}", OBJECT_ORDERS.join(", "))),
            Kind::Move => (!one_of(&MOVES))
                .then(|| format!("entrance.{key} must be one of {}", MOVES.join(", "))),
            Kind::ObjectMove => (!one_of(&OBJECT_MOVES))
                .then(|| format!("entrance.{key} must be one of {}", OBJECT_MOVES.join(", "))),
            Kind::Flag => {
                (!v.is_boolean()).then(|| format!("entrance.{key} must be true or false"))
            }
            Kind::Seed => (!n.is_some_and(is_integer))
                .then(|| format!("entrance.{key} must be a whole number")),
        };
        if let Some(p) = problem {
            add(p);
        }
    }
}

/// Whether a field's value is its default.
pub fn is_default(key: &str, value: &Value) -> bool {
    FIELDS
        .iter()
        .find(|(k, _, _)| *k == key)
        .is_some_and(|(_, _, d)| match d {
            Default::Num(n) => value.as_f64() == Some(*n),
            Default::Str(s) => value.as_str() == Some(*s),
            Default::Bool(b) => value.as_bool() == Some(*b),
        })
}

/// Seconds, art pixels and scales: every field named.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub start: f64,
    pub ground_delay: f64,
    pub ground_span: f64,
    pub cascade: f64,
    pub ground: String,
    pub objects: String,
    pub with: bool,
    pub object_lead: f64,
    pub object_span: f64,
    pub lag: f64,
    pub mv: String,
    pub object_move: String,
    pub duration: f64,
    pub peak: f64,
    pub rise: f64,
    pub overshoot: f64,
    pub pop_from: f64,
    pub pop_peak: f64,
    pub object_duration: f64,
    pub object_peak: f64,
    pub object_rise: f64,
    pub object_overshoot: f64,
    pub object_pop_from: f64,
    pub object_pop_peak: f64,
    pub shadow_duration: f64,
    pub sky_duration: f64,
    pub sky_drift: f64,
    pub seed: f64,
}

impl Timing {
    /// The timing an entrance gives (fields left out take pixellab's
    /// defaults), in seconds.
    pub fn of(entrance: &Value) -> Timing {
        let get = |key: &str| -> Value {
            match entrance.get(key) {
                Some(v) => v.clone(),
                None => FIELDS
                    .iter()
                    .find(|(k, _, _)| *k == key)
                    .map(|(_, _, d)| d.to_value())
                    .unwrap_or(Value::Null),
            }
        };
        let ms = |key: &str| get(key).as_f64().unwrap_or(0.0) / 1000.0;
        let n = |key: &str| get(key).as_f64().unwrap_or(0.0);
        let s = |key: &str| get(key).as_str().unwrap_or("").to_string();
        Timing {
            start: ms("start"),
            ground_delay: ms("groundDelay"),
            ground_span: ms("groundSpan"),
            cascade: ms("cascade"),
            ground: s("ground"),
            objects: s("objects"),
            with: get("with").as_bool().unwrap_or(true),
            object_lead: ms("objectLead"),
            object_span: ms("objectSpan"),
            lag: ms("lag"),
            mv: s("move"),
            object_move: s("objectMove"),
            duration: ms("duration"),
            peak: n("peak"),
            rise: n("rise"),
            overshoot: n("overshoot"),
            pop_from: n("popFrom"),
            pop_peak: n("popPeak"),
            object_duration: ms("objectDuration"),
            object_peak: n("objectPeak"),
            object_rise: n("objectRise"),
            object_overshoot: n("objectOvershoot"),
            object_pop_from: n("objectPopFrom"),
            object_pop_peak: n("objectPopPeak"),
            shadow_duration: ms("shadowDuration"),
            sky_duration: ms("skyDuration"),
            sky_drift: n("skyDrift"),
            seed: n("seed"),
        }
    }

    /// The site's timing object (seconds) as an entrance (ms, only the fields
    /// that differ from the default), as `pixellab scene entrance set
    /// --timing` reads it.
    pub fn entrance_from_site(site: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let mut out = Map::new();
        for (key, v) in site {
            let Some((_, kind, _)) = FIELDS.iter().find(|(k, _, _)| k == key) else {
                return Err(format!("unknown timing field {}", qs(key)));
            };
            let value = match kind {
                Kind::Ms => {
                    let s = v
                        .as_f64()
                        .ok_or_else(|| format!("timing.{key} must be a number of seconds"))?;
                    num(js_round(s * 1000.0))
                }
                _ => v.clone(),
            };
            out.insert(key.clone(), value);
        }
        Ok(out.into_iter().filter(|(k, v)| !is_default(k, v)).collect())
    }
}

/// Reads an entrance configuration (ms), reporting pixellab's problems.
pub fn timing_from_entrance(entrance: &Value) -> Result<Timing, Vec<String>> {
    let mut problems = Vec::new();
    validate(entrance, &mut |p| problems.push(p));
    if problems.is_empty() {
        Ok(Timing::of(entrance))
    } else {
        Err(problems)
    }
}

/// Reads the site's seconds form, as `--timing` does.
pub fn timing_from_site(site: &Value) -> Result<Timing, Vec<String>> {
    let Some(o) = site.as_object() else {
        return Err(vec![
            "siteTiming is the site's TIMING object (seconds)".into()
        ]);
    };
    let entrance = Timing::entrance_from_site(o).map_err(|m| vec![m])?;
    timing_from_entrance(&Value::Object(entrance))
}

// ------------------------------------------------------------------ pieces

/// A piece as the schedule reads it: its role, grid column and row
/// (ground), and foot in scene pixels. A missing number is None.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub enter: String,
    pub col: Option<f64>,
    pub row: Option<f64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

/// One piece's entrance: when it starts and how it moves (seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Entrance {
    /// `float`, `drop`, `pop`, `stepped`, `fade` or `drift` (the sky).
    pub mv: String,
    pub delay: f64,
    pub duration: f64,
    /// Share of the move spent reaching the overshoot (0 for a fade).
    pub peak: f64,
    /// Travel distance in art pixels.
    pub rise: f64,
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    u: f64,
    v: f64,
    x: f64,
    y: f64,
}

/// JavaScript's `x | 0`.
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc().rem_euclid(4294967296.0);
    m as u32 as i32
}

/// pixellab's small seeded random in [0, 1) (`random`).
pub fn random(seed: f64, i: f64) -> f64 {
    let mut t = to_int32(seed * 9301.0 + i * 49297.0 + 233280.0);
    t = (t ^ ((t as u32) >> 15) as i32).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ ((t as u32) >> 7) as i32).wrapping_mul(t | 61));
    ((t ^ ((t as u32) >> 14) as i32) as u32) as f64 / 4294967296.0
}

/// `Math.min(...list)` and `Math.max(...list)`: NaN if any is NaN.
fn js_min(list: &[f64]) -> f64 {
    list.iter().fold(f64::INFINITY, |m, &x| {
        if m.is_nan() || x.is_nan() {
            f64::NAN
        } else {
            m.min(x)
        }
    })
}

fn js_max(list: &[f64]) -> f64 {
    list.iter().fold(f64::NEG_INFINITY, |m, &x| {
        if m.is_nan() || x.is_nan() {
            f64::NAN
        } else {
            m.max(x)
        }
    })
}

/// Scores mapped onto 0-1 (all 0 when flat).
pub fn normalize(scores: &[f64]) -> Vec<f64> {
    let (min, max) = (js_min(scores), js_max(scores));
    scores
        .iter()
        .map(|s| {
            if max > min {
                (s - min) / (max - min)
            } else {
                0.0
            }
        })
        .collect()
}

/// Every piece's place (`cells`).
fn cells(pieces: &[Piece]) -> Vec<Option<Cell>> {
    let anchor = pieces.iter().find(|p| {
        p.enter == "ground" && p.col.is_some() && p.row.is_some() && p.x.is_some() && p.y.is_some()
    });
    let (ox, oy) = match anchor {
        Some(a) => {
            let (col, row) = (a.col.unwrap_or(0.0), a.row.unwrap_or(0.0));
            (
                a.x.unwrap_or(0.0) - (col - row) * HALF_W,
                a.y.unwrap_or(0.0) - (col + row) * HALF_H,
            )
        }
        None => (0.0, 0.0),
    };
    pieces
        .iter()
        .map(|p| match p.enter.as_str() {
            "ground" => {
                let (u, v) = (p.col.unwrap_or(0.0), p.row.unwrap_or(0.0));
                Some(Cell {
                    u,
                    v,
                    x: p.x.unwrap_or((u - v) * HALF_W + ox),
                    y: p.y.unwrap_or((u + v) * HALF_H + oy),
                })
            }
            "object" => {
                let (sx, sy) = (p.x.unwrap_or(0.0), p.y.unwrap_or(oy));
                let (a, b) = ((sx - ox) / HALF_W, (sy - oy) / HALF_H);
                Some(Cell {
                    u: (a + b) / 2.0,
                    v: (b - a) / 2.0,
                    x: sx,
                    y: sy,
                })
            }
            _ => None,
        })
        .collect()
}

/// Every piece's place as (u, v, x, y): ground at its column and row,
/// objects at their foot; None for shadow and sky.
pub fn places(pieces: &[Piece]) -> Vec<Option<[f64; 4]>> {
    cells(pieces)
        .into_iter()
        .map(|c| c.map(|c| [c.u, c.v, c.x, c.y]))
        .collect()
}

/// The score each order gives a piece (`score`).
fn score(order: &str, c: Cell, i: usize, t: &Timing, center: (f64, f64)) -> f64 {
    let (cx, cy) = center;
    let du = ((c.x - cx) / HALF_W + (c.y - cy) / HALF_H) / 2.0;
    let dv = ((c.y - cy) / HALF_H - (c.x - cx) / HALF_W) / 2.0;
    match order {
        "rows" => -c.v,
        "cols" => c.u,
        "back-front" => c.y,
        "front-back" => -c.y,
        "sweep" | "ltr" => c.x,
        "center" => libm::hypot(du, dv),
        "edges" => -libm::hypot(du, dv),
        "clock" => {
            let a = libm::atan2(c.y - cy, c.x - cx);
            let tau = 2.0 * std::f64::consts::PI;
            (((a - std::f64::consts::PI) % tau) + tau) % tau
        }
        "checker" => (((js_round(c.u) + js_round(c.v)) % 2.0) + 2.0) % 2.0,
        "random" => random(t.seed, i as f64),
        _ => 0.0,
    }
}

/// How one kind of piece moves (`motionOf`): (move, duration, peak, rise).
fn motion_of(t: &Timing, object: bool) -> (String, f64, f64, f64) {
    if !object || t.object_move == "same" {
        return (t.mv.clone(), t.duration, t.peak, t.rise);
    }
    (
        t.object_move.clone(),
        t.object_duration,
        t.object_peak,
        t.object_rise,
    )
}

/// What the schedule works out besides the entrances: the order score each
/// ground and object piece got (normalized, 0-1).
#[derive(Clone, Debug)]
pub struct Schedule {
    pub entrances: Vec<Option<Entrance>>,
    /// The order's own normalized score per piece (None for shadow and sky).
    pub order: Vec<Option<f64>>,
}

/// Each piece's entrance, in the order given, or None for a piece it
/// doesn't know (`schedule`).
pub fn schedule(pieces: &[Piece], t: &Timing) -> Schedule {
    let place = cells(pieces);
    let ground: Vec<usize> = (0..pieces.len())
        .filter(|&i| pieces[i].enter == "ground")
        .collect();
    let objects: Vec<usize> = (0..pieces.len())
        .filter(|&i| pieces[i].enter == "object")
        .collect();
    let at = |i: usize| place[i].expect("a ground or object piece has a place");
    let xs: Vec<f64> = ground.iter().map(|&i| at(i).x).collect();
    let ys: Vec<f64> = ground.iter().map(|&i| at(i).y).collect();
    let center = if ground.is_empty() {
        (0.0, 0.0)
    } else {
        (
            (js_min(&xs) + js_max(&xs)) / 2.0,
            (js_min(&ys) + js_max(&ys)) / 2.0,
        )
    };

    let first_ground = t.start + t.ground_delay;
    let ground_t = normalize(
        &ground
            .iter()
            .map(|&i| score(&t.ground, at(i), i, t, center))
            .collect::<Vec<_>>(),
    );
    let across = normalize(&xs);
    let mut delays: Vec<Option<f64>> = vec![None; pieces.len()];
    let mut order: Vec<Option<f64>> = vec![None; pieces.len()];
    for (k, &i) in ground.iter().enumerate() {
        delays[i] = Some(first_ground + ground_t[k] * t.ground_span + across[k] * t.cascade);
        order[i] = Some(ground_t[k]);
    }
    let mut last = vec![first_ground];
    last.extend(ground.iter().map(|&i| delays[i].unwrap_or(first_ground)));
    let ground_end = js_max(&last) + t.duration;

    let mut object_start = ground_end - t.object_lead;
    if t.with {
        // Each object rides in with the ground piece nearest its foot.
        for &i in &objects {
            let mut best: Option<usize> = None;
            let mut near = f64::INFINITY;
            for &g in &ground {
                let d = libm::hypot(at(g).u - at(i).u, at(g).v - at(i).v);
                if d < near {
                    best = Some(g);
                    near = d;
                }
            }
            delays[i] = Some(best.and_then(|b| delays[b]).unwrap_or(first_ground) + t.lag);
            order[i] = Some(best.and_then(|b| order[b]).unwrap_or(0.0));
        }
        object_start = first_ground + t.ground_span / 2.0;
    } else {
        let ord = if t.objects == "same" {
            t.ground.as_str()
        } else {
            t.objects.as_str()
        };
        let object_t = normalize(
            &objects
                .iter()
                .map(|&i| score(ord, at(i), i, t, center))
                .collect::<Vec<_>>(),
        );
        for (k, &i) in objects.iter().enumerate() {
            delays[i] = Some(object_start + object_t[k] * t.object_span);
            order[i] = Some(object_t[k]);
        }
    }

    let moves = [motion_of(t, false), motion_of(t, true)];
    let entrances = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| match p.enter.as_str() {
            "shadow" => Some(Entrance {
                mv: "fade".into(),
                delay: t.start,
                duration: t.shadow_duration,
                peak: 0.0,
                rise: 0.0,
            }),
            "ground" | "object" => {
                let (mv, duration, peak, rise) = &moves[usize::from(p.enter == "object")];
                Some(Entrance {
                    mv: mv.clone(),
                    delay: delays[i].unwrap_or(first_ground),
                    duration: *duration,
                    peak: if mv == "fade" { 0.0 } else { *peak },
                    rise: *rise,
                })
            }
            "sky" => Some(Entrance {
                mv: "drift".into(),
                delay: object_start,
                duration: t.sky_duration,
                peak: 0.0,
                rise: t.sky_drift,
            }),
            _ => None,
        })
        .collect();
    Schedule { entrances, order }
}

/// When the last piece has settled, in seconds from the start (`settledBy`).
pub fn settled_by(plan: &[Option<Entrance>]) -> f64 {
    plan.iter()
        .flatten()
        .fold(0.0, |m: f64, e| m.max(e.delay + e.duration))
}

/// When a piece's exit runs, in seconds after the exit begins: the
/// entrance backwards (pixellab's `exit_pose_at`), from
/// `settledBy - (delay + duration)` to `settledBy - delay`.
pub fn exit_window(e: &Entrance, settled: f64) -> (f64, f64) {
    (settled - (e.delay + e.duration), settled - e.delay)
}

// ------------------------------------------------------------------ scenes

/// A scene's entrance as cozy-score scores it: the pieces and their
/// schedule (seconds), however it was given.
#[derive(Clone, Debug)]
pub struct Scene {
    pub name: Option<String>,
    /// "from", "pieces", "grid" or "scene".
    pub source: &'static str,
    pub pieces: Vec<Piece>,
    pub timing: Timing,
    pub entrances: Vec<Option<Entrance>>,
    pub order: Vec<Option<f64>>,
    pub settled_by: f64,
    /// True for a synthetic grid: the times only approximate a real scene.
    pub approximate: bool,
}

impl Scene {
    pub fn from_pieces(pieces: Vec<Piece>, timing: Timing, source: &'static str) -> Scene {
        let sch = schedule(&pieces, &timing);
        let settled = settled_by(&sch.entrances);
        Scene {
            name: None,
            source,
            pieces,
            timing,
            entrances: sch.entrances,
            order: sch.order,
            settled_by: settled,
            approximate: false,
        }
    }
}

fn opt_num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// Reads a list of pieces without times (`role` or `enter`, `col`, `row`,
/// `x`, `y`), the shape of the `pieces` lists in pixellab's parity fixture.
pub fn read_pieces(list: &Value, ctx: &str, errors: &mut Vec<String>) -> Vec<Piece> {
    let Some(arr) = list.as_array() else {
        errors.push(format!(
            "{ctx}: pieces must be a list of {{ role, col, row, x, y }}"
        ));
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, p) in arr.iter().enumerate() {
        let Some(o) = p.as_object() else {
            errors.push(format!("{ctx}: pieces[{i}] must be an object"));
            continue;
        };
        for k in o.keys() {
            if !matches!(
                k.as_str(),
                "role"
                    | "enter"
                    | "col"
                    | "row"
                    | "x"
                    | "y"
                    | "path"
                    | "delay"
                    | "duration"
                    | "move"
            ) {
                errors.push(format!(
                    "{ctx}: pieces[{i}]: unknown property {} (expected role, col, row, x, y)",
                    qs(k)
                ));
            }
        }
        let role = o
            .get("role")
            .or_else(|| o.get("enter"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !ROLES.contains(&role) {
            errors.push(format!(
                "{ctx}: pieces[{i}]: role must be one of {}",
                ROLES.join(", ")
            ));
            continue;
        }
        for k in ["col", "row", "x", "y"] {
            if let Some(v) = o.get(k) {
                if !v.is_null() && v.as_f64().is_none_or(|n| !n.is_finite()) {
                    errors.push(format!("{ctx}: pieces[{i}]: {k} must be a number"));
                }
            }
        }
        out.push(Piece {
            enter: role.to_string(),
            col: opt_num(p, "col"),
            row: opt_num(p, "row"),
            x: opt_num(p, "x"),
            y: opt_num(p, "y"),
        });
    }
    out
}

/// Reads the output of `pixellab scene entrance show <scene> --json`:
/// `{ scene, entrance, timing, settledBy, pieces: [{ role, path, col?, row?,
/// x?, y?, delay, duration, move }] }` in ms. Its per-piece times are used
/// as they are.
pub fn scene_from_show(v: &Value, ctx: &str) -> Result<Scene, Vec<String>> {
    let mut errors = Vec::new();
    if v.get("entrance").is_some_and(Value::is_null) {
        let name = v
            .get("scene")
            .and_then(Value::as_str)
            .unwrap_or("the scene");
        return Err(vec![format!(
            "{ctx}: {name} has no entrance (pixellab scene entrance set {name} gives it one)"
        )]);
    }
    let timing_v = v
        .get("timing")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let timing = match timing_from_entrance(&timing_v) {
        Ok(t) => t,
        Err(e) => {
            errors.extend(e.into_iter().map(|m| format!("{ctx}: {m}")));
            Timing::of(&Value::Object(Map::new()))
        }
    };
    let Some(list) = v.get("pieces").and_then(Value::as_array) else {
        errors.push(format!(
            "{ctx}: not the output of `pixellab scene entrance show --json` (no pieces)"
        ));
        return Err(errors);
    };
    let pieces = read_pieces(&Value::Array(list.clone()), ctx, &mut errors);
    let mut entrances = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let delay = opt_num(p, "delay");
        let duration = opt_num(p, "duration");
        let mv = p.get("move").and_then(Value::as_str).map(str::to_string);
        match (delay, duration, mv) {
            (Some(d), Some(du), Some(mv)) => {
                let object = p.get("role").and_then(Value::as_str) == Some("object");
                let (_, _, peak, rise) = motion_of(&timing, object);
                let (peak, rise) = match mv.as_str() {
                    "fade" => (0.0, 0.0),
                    "drift" => (0.0, timing.sky_drift),
                    _ => (peak, rise),
                };
                entrances.push(Some(Entrance {
                    mv,
                    delay: d / 1000.0,
                    duration: du / 1000.0,
                    peak,
                    rise,
                }));
            }
            _ => {
                errors.push(format!(
                    "{ctx}: pieces[{i}] has no delay, duration and move"
                ));
                entrances.push(None);
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // The order scores come from the port; the times stay pixellab's own.
    let order = schedule(&pieces, &timing).order;
    let settled = opt_num(v, "settledBy")
        .map(|ms| ms / 1000.0)
        .unwrap_or_else(|| settled_by(&entrances));
    Ok(Scene {
        name: v.get("scene").and_then(Value::as_str).map(str::to_string),
        source: "from",
        pieces,
        timing,
        entrances,
        order,
        settled_by: settled,
        approximate: false,
    })
}

/// A synthetic isometric grid: `cols` x `rows` blocks, block (u, v) at
/// x = (u - v) 22, y = (u + v) 11, `objects` objects on seeded distinct
/// cells, and a shadow and sky if asked for.
pub fn grid_pieces(
    cols: usize,
    rows: usize,
    objects: usize,
    shadow: bool,
    sky: bool,
    rng: &mut Rng,
) -> Vec<Piece> {
    let mut out = Vec::new();
    if shadow {
        out.push(Piece {
            enter: "shadow".into(),
            col: None,
            row: None,
            x: None,
            y: None,
        });
    }
    let cell = |u: usize, v: usize| ((u as f64 - v as f64) * HALF_W, (u + v) as f64 * HALF_H);
    for v in 0..rows {
        for u in 0..cols {
            let (x, y) = cell(u, v);
            out.push(Piece {
                enter: "ground".into(),
                col: Some(u as f64),
                row: Some(v as f64),
                x: Some(x),
                y: Some(y),
            });
        }
    }
    // Distinct cells: a seeded partial shuffle of every cell.
    let mut all: Vec<(usize, usize)> = (0..rows)
        .flat_map(|v| (0..cols).map(move |u| (u, v)))
        .collect();
    let n = objects.min(all.len());
    for k in 0..n {
        let j = k + rng.below(all.len() - k);
        all.swap(k, j);
        let (u, v) = all[k];
        let (x, y) = cell(u, v);
        out.push(Piece {
            enter: "object".into(),
            col: None,
            row: None,
            x: Some(x),
            y: Some(y),
        });
    }
    if sky {
        out.push(Piece {
            enter: "sky".into(),
            col: None,
            row: None,
            x: None,
            y: None,
        });
    }
    out
}

/// Runs the read-only `pixellab scene entrance show <scene> --json
/// --document <file>`, when pixellab is on PATH.
pub fn show_with_pixellab(scene: &str, document: &Path) -> Result<Value, String> {
    let out = std::process::Command::new("pixellab")
        .args(["scene", "entrance", "show", scene, "--json", "--document"])
        .arg(document)
        .output()
        .map_err(|_| {
            format!(
                "\"scene\": \"{scene}\" needs pixellab on PATH; without it, export the entrance with \
                 `pixellab scene entrance show {scene} --json > {scene}.json` and pass \"from\": \"{scene}.json\" instead"
            )
        })?;
    if !out.status.success() {
        return Err(format!(
            "pixellab scene entrance show {scene} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("pixellab scene entrance show {scene} gave no JSON: {e}"))
}

// ------------------------------------------------------------------ sequences

/// A shot: a scene, played for `length` ms, then held for `hold` ms.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub scene: String,
    pub length: f64,
    pub entrance: bool,
    pub hold: f64,
}

/// A transition at a join.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub kind: String,
    pub length: f64,
    pub direction: String,
    pub gap: f64,
}

impl Transition {
    /// Whether it overlaps the end of one shot with the start of the next.
    pub fn overlaps(&self) -> bool {
        matches!(self.kind.as_str(), "fade" | "dip" | "slide" | "wipe")
    }
}

/// A sequence, read leniently as pixellab reads one.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub shots: Vec<Shot>,
    pub transitions: Vec<Transition>,
}

fn num_or(v: Option<&Value>, or: f64) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(or)
}

fn text_or(v: Option<&Value>, or: &str) -> String {
    v.and_then(Value::as_str).unwrap_or(or).to_string()
}

impl Sequence {
    pub fn of(v: &Value) -> Sequence {
        let shots = v
            .get("shots")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let transitions = v
            .get("transitions")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        Sequence {
            shots: shots
                .iter()
                .map(|s| Shot {
                    scene: text_or(s.get("scene"), ""),
                    length: num_or(s.get("length"), 0.0),
                    entrance: s.get("entrance").and_then(Value::as_bool).unwrap_or(false),
                    hold: num_or(s.get("hold"), 0.0),
                })
                .collect(),
            transitions: transitions
                .iter()
                .map(|t| Transition {
                    kind: text_or(t.get("kind"), "cut"),
                    length: num_or(t.get("length"), 0.0),
                    direction: text_or(t.get("direction"), "left"),
                    gap: num_or(t.get("gap"), 0.0),
                })
                .collect(),
        }
    }
}

/// Where a shot starts and ends on the sequence's time (ms), the end
/// including its hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

/// The shots laid out on the sequence's time: end to end, a `fade`, `dip`,
/// `slide` or `wipe` overlapping the two shots by its length, a `blocks`
/// join adding its gap (`spans`).
pub fn spans(seq: &Sequence) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut at = 0.0;
    for (i, shot) in seq.shots.iter().enumerate() {
        if i > 0 {
            let t = seq.transitions.get(i - 1);
            let prev = out[i - 1].end;
            at = match t {
                Some(t) if t.overlaps() => prev - t.length,
                Some(t) if t.kind == "blocks" => prev + t.gap,
                _ => prev,
            };
        }
        out.push(Span {
            start: at,
            end: at + shot.length + shot.hold,
        });
    }
    out
}

/// Whether a shot opens with its scene's entrance: its own `entrance`, or a
/// `blocks` join before it (`opens_with_entrance`).
pub fn opens_with_entrance(seq: &Sequence, i: usize) -> bool {
    seq.shots[i].entrance
        || (i > 0
            && seq
                .transitions
                .get(i - 1)
                .is_some_and(|t| t.kind == "blocks"))
}

// ------------------------------------------------------------------ drafting a plan

/// `to` written relative to the directory `from`.
pub fn relative_path(to: &Path, from: &Path) -> std::path::PathBuf {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (to_a, from_a) = (abs(to), abs(from));
    let tc: Vec<_> = to_a.components().collect();
    let fc: Vec<_> = from_a.components().collect();
    let common = tc.iter().zip(&fc).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return to_a;
    }
    let mut out = std::path::PathBuf::new();
    for _ in common..fc.len() {
        out.push("..");
    }
    for c in &tc[common..] {
        out.push(c);
    }
    out
}

fn secs(ms: f64) -> Value {
    num(ms / 1000.0)
}

/// What `from-pixellab` drafts a plan from.
pub struct SequenceDraft<'a> {
    /// The output of `pixellab sequence show <name> --json`.
    pub sequence: &'a Value,
    /// The directory of `pixellab scene entrance show --json` outputs.
    pub entrances: &'a Path,
    /// Section types by shot index (0-based); the rest take the defaults.
    pub types: &'a [(usize, String)],
    /// Where the plan will be written (for relative `from` paths).
    pub plan_dir: &'a Path,
    pub feel: Option<&'a str>,
}

/// Drafts a plan from a pixellab sequence: one section per shot, an
/// entrance where a shot opens with one, an exit before each `blocks`
/// join, and a sound for every other join. Every time is checked against
/// `spans()` and explained in the plan's `notes`.
pub fn draft_from_sequence(d: &SequenceDraft) -> Result<Value, Vec<String>> {
    let v = d.sequence;
    let mut errors = Vec::new();
    let mut notes: Vec<Value> = Vec::new();
    let shots_v = v
        .get("shots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if shots_v.is_empty() {
        return Err(vec![
            "the sequence has no shots (pass the output of `pixellab sequence show <name> --json`)"
                .into(),
        ]);
    }
    let seq = Sequence::of(v);
    let sp = spans(&seq);
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("the sequence");
    notes.push(Value::String(format!(
        "Drafted by cozy-score from-pixellab from {name}: {} shots, {} ms. Times are pixellab's spans() (ms), shown here in seconds.",
        shots_v.len(),
        sp.last().map_or(0.0, |s| s.end)
    )));
    // Cross-check the given times against spans().
    for (i, (shot, span)) in shots_v.iter().zip(&sp).enumerate() {
        let (gs, ge) = (opt_num(shot, "start"), opt_num(shot, "end"));
        if gs.is_some_and(|x| (x - span.start).abs() > 1e-6)
            || ge.is_some_and(|x| (x - span.end).abs() > 1e-6)
        {
            errors.push(format!(
                "shot {}: the sequence says {}-{} ms but spans() gives {}-{} ms",
                i + 1,
                gs.unwrap_or(f64::NAN),
                ge.unwrap_or(f64::NAN),
                span.start,
                span.end
            ));
        }
    }
    if let Some(len) = opt_num(v, "length") {
        let ours = sp.last().map_or(0.0, |s| s.end);
        if (len - ours).abs() > 1e-6 {
            errors.push(format!(
                "the sequence's length is {len} ms but spans() ends at {ours} ms"
            ));
        }
    }
    // Entrance files, for scenes that enter or exit.
    let n = seq.shots.len();
    let exits_before: Vec<bool> = (0..n)
        .map(|i| seq.transitions.get(i).is_some_and(|t| t.kind == "blocks"))
        .collect();
    let mut settled: Vec<Option<f64>> = vec![None; n];
    let mut from_of: Vec<Option<String>> = vec![None; n];
    for i in 0..n {
        if !(opens_with_entrance(&seq, i) || exits_before[i]) {
            continue;
        }
        let scene = &seq.shots[i].scene;
        let file = d.entrances.join(format!("{scene}.json"));
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!(
                    "shot {}: cannot read {} ({e}); export it with `pixellab scene entrance show {scene} --json > {}`",
                    i + 1,
                    file.display(),
                    file.display()
                ));
                continue;
            }
        };
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                errors.push(format!("{} is not JSON: {e}", file.display()));
                continue;
            }
        };
        match scene_from_show(&parsed, &file.display().to_string()) {
            Ok(sc) => {
                settled[i] = Some(sc.settled_by * 1000.0);
                from_of[i] = Some(
                    relative_path(&file, d.plan_dir)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
            Err(e) => errors.extend(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // Sections: one per shot; an overlap splits at its midpoint, a blocks
    // gap belongs to the following shot.
    let mut starts = vec![0.0; n];
    for i in 1..n {
        let t = &seq.transitions.get(i - 1);
        starts[i] = match t {
            Some(t) if t.overlaps() => (sp[i].start + sp[i - 1].end) / 2.0,
            Some(t) if t.kind == "blocks" => sp[i - 1].end,
            _ => sp[i].start,
        };
    }
    let mut sections = Vec::new();
    for i in 0..n {
        let end = if i + 1 < n { starts[i + 1] } else { sp[i].end };
        let kind = d
            .types
            .iter()
            .find(|(k, _)| *k == i)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| {
                if i == 0 {
                    "intro".into()
                } else if i + 1 == n {
                    "outro-end-screen".into()
                } else {
                    "visual-demo".into()
                }
            });
        sections.push(serde_json::json!([kind, [secs(starts[i]), secs(end)]]));
        let why = if i == 0 {
            "starts the sequence".to_string()
        } else {
            let t = &seq.transitions[i - 1];
            match t.kind.as_str() {
                "fade" | "dip" | "slide" | "wipe" => format!(
                    "starts at the midpoint of the {} ms {} overlap ({}-{} ms)",
                    t.length,
                    t.kind,
                    sp[i].start,
                    sp[i - 1].end
                ),
                "blocks" => format!(
                    "starts where shot {} ends ({} ms), taking the {} ms blocks gap",
                    i,
                    sp[i - 1].end,
                    t.gap
                ),
                _ => format!("starts at the cut ({} ms)", sp[i].start),
            }
        };
        notes.push(Value::String(format!(
            "Section {} ({kind}): shot {} ({}), spans() {}-{} ms; {why}; ends at {} ms.",
            i + 1,
            i + 1,
            seq.shots[i].scene,
            sp[i].start,
            sp[i].end,
            end
        )));
    }

    // Events.
    let mut events = Vec::new();
    for i in 0..n {
        if opens_with_entrance(&seq, i) {
            let at = sp[i].start;
            events
                .push(serde_json::json!({"kind": "entrance", "from": from_of[i], "at": secs(at)}));
            notes.push(Value::String(format!(
                "Entrance of {} at {} ms: shot {} opens with its entrance ({}); settles {} ms later.",
                seq.shots[i].scene,
                at,
                i + 1,
                if seq.shots[i].entrance { "its own entrance flag" } else { "a blocks join before it" },
                settled[i].unwrap_or(0.0)
            )));
        }
        if i + 1 < n {
            let t = &seq.transitions[i];
            let join = sp[i + 1].start;
            match t.kind.as_str() {
                "blocks" => {
                    let s = settled[i].unwrap_or(0.0);
                    let at = sp[i].start + seq.shots[i].length + seq.shots[i].hold - s;
                    events.push(
                        serde_json::json!({"kind": "exit", "from": from_of[i], "at": secs(at)}),
                    );
                    notes.push(Value::String(format!(
                        "Exit of {} at {at} ms = shot start {} + length {} + hold {} - settledBy {s}; it ends with a pop at {} ms, before the blocks join.",
                        seq.shots[i].scene, sp[i].start, seq.shots[i].length, seq.shots[i].hold, sp[i].end
                    )));
                }
                "cut" => {
                    events.push(
                        serde_json::json!({"kind": "hit", "at": secs(join), "strength": "soft"}),
                    );
                    notes.push(Value::String(format!(
                        "Soft hit at the cut after shot {} ({join} ms).",
                        i + 1
                    )));
                }
                "slide" | "wipe" => {
                    let (a, b) = (sp[i + 1].start, sp[i].end);
                    let dir = if matches!(t.direction.as_str(), "right" | "down") {
                        "down"
                    } else {
                        "up"
                    };
                    let count = ((b - a) / 100.0).round().clamp(4.0, 16.0) as usize;
                    events.push(serde_json::json!({
                        "kind": "run", "direction": dir, "start": secs(a), "count": count, "end": secs(b)
                    }));
                    notes.push(Value::String(format!(
                        "Run {dir} across the {} {} after shot {} ({a}-{b} ms).",
                        t.direction,
                        t.kind,
                        i + 1
                    )));
                }
                "fade" | "dip" => {
                    let (a, b) = (sp[i + 1].start, sp[i].end);
                    events.push(
                        serde_json::json!({"kind": "riser", "start": secs(a), "end": secs(b)}),
                    );
                    notes.push(Value::String(format!(
                        "Riser swell across the {} after shot {} ({a}-{b} ms).",
                        t.kind,
                        i + 1
                    )));
                }
                _ => {}
            }
        }
    }
    let mut plan = Map::new();
    if let Some(f) = d.feel {
        plan.insert("feel".into(), Value::String(f.into()));
    }
    plan.insert("notes".into(), Value::Array(notes));
    plan.insert("sections".into(), Value::Array(sections));
    plan.insert("events".into(), Value::Array(events));
    Ok(Value::Object(plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_int32_wraps_like_javascript() {
        assert_eq!(to_int32(4294967296.0 + 5.0), 5);
        assert_eq!(to_int32(2147483648.0), -2147483648);
        assert_eq!(to_int32(-1.0), -1);
    }

    #[test]
    fn js_round_rounds_halves_up() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.49), 0.0);
    }

    #[test]
    fn spans_follow_the_joins() {
        let seq = Sequence::of(&serde_json::json!({
            "shots": [
                {"scene": "a", "length": 3000},
                {"scene": "b", "length": 2000, "hold": 500},
                {"scene": "c", "length": 2000},
                {"scene": "d", "length": 1000}
            ],
            "transitions": [{"kind": "fade", "length": 400}, {"kind": "blocks", "gap": 300}, {"kind": "cut"}]
        }));
        let s = spans(&seq);
        assert_eq!(
            s[0],
            Span {
                start: 0.0,
                end: 3000.0
            }
        );
        assert_eq!(
            s[1],
            Span {
                start: 2600.0,
                end: 5100.0
            }
        );
        assert_eq!(
            s[2],
            Span {
                start: 5400.0,
                end: 7400.0
            }
        );
        assert_eq!(
            s[3],
            Span {
                start: 7400.0,
                end: 8400.0
            }
        );
        assert!(opens_with_entrance(&seq, 2));
        assert!(!opens_with_entrance(&seq, 3));
    }
}
