//! Plans with events: parsing, run and tick timing, contours, the tempo
//! search, quantizing, transitions, allow-lists, keys and endings. Checked
//! on note data, apart from the endings.

use cozy_score::arrange::Art;
use cozy_score::events::{
    Contour, EventKind, EventNote, Level, ParseContext, PitchMode, Quant, RiserKind, RunTiming,
    ScaleKind, Strength, Sync, TickPitch, TickTiming,
};
use cozy_score::harmony::{fold_bass, key_shift, parse_key, Key, BASS_HI, BASS_LO};
use cozy_score::plan::{parse_plan_with, Plan};
use cozy_score::render::{
    arrange_plan, event_voices, render_with, FadeOut, Options, Planned, Request,
};
use cozy_score::resolve::candidates_in;
use cozy_score::types::{Feel, Instrument, Role, VideoType};
use std::path::Path;

const STING: &str = include_str!("../plans/startup-sting.json");

fn parse(text: &str) -> Result<Plan, Vec<String>> {
    parse_plan_with(
        text,
        &ParseContext {
            base: Path::new(env!("CARGO_MANIFEST_DIR")),
            document: None,
            seed: 7,
        },
    )
}

fn options(p: &Plan) -> Options {
    Options {
        events: p.events.clone(),
        bpm: p.bpm,
        key: p.key,
        fade_out: FadeOut::Auto,
        tail: 0.0,
        fps: p.fps,
    }
}

fn planned_with(text: &str, instruments: &[Instrument], edit: impl Fn(&mut Options)) -> Planned {
    let p = parse(text).unwrap();
    let mut o = options(&p);
    edit(&mut o);
    arrange_plan(
        &Request {
            specs: &p.sections,
            feel: p.feel.unwrap_or(Feel::Spring),
            seed: 7,
            instruments: instruments.to_vec(),
            stems: false,
            gain_from: None,
        },
        &o,
    )
    .unwrap()
}

fn planned(text: &str) -> Planned {
    planned_with(text, &Instrument::ALL, |_| {})
}

fn notes<'a>(p: &'a Planned, event: usize, what: &str) -> Vec<&'a EventNote> {
    p.realized
        .notes
        .iter()
        .filter(|n| n.event == event && n.what == what)
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ------------------------------------------------------------------ parsing

#[test]
fn the_array_form_still_parses_and_is_marked_legacy() {
    let p = parse(r#"[["intro","0:00-0:08"],["explain","0:08-0:20"]]"#).unwrap();
    assert!(p.legacy && p.events.is_empty() && p.feel.is_none());
    assert_eq!(p.sections.len(), 2);
}

#[test]
fn the_object_form_carries_feel_tempo_key_and_events() {
    let p = parse(STING).unwrap();
    assert!(!p.legacy);
    assert_eq!(p.feel, Some(Feel::Chip));
    assert_eq!(p.bpm, Some(150.0));
    assert_eq!(p.key, Some(parse_key("C").unwrap()));
    assert_eq!(p.sections.len(), 3);
    assert_eq!(p.sections[0].kind, VideoType::Build);
    assert_eq!(
        p.sections[0].instruments,
        Some(vec![
            Instrument::Mallet,
            Instrument::SynthPluck,
            Instrument::Riser
        ])
    );
    assert_eq!(p.events.len(), 4);
    let kinds: Vec<&str> = p.events.iter().map(|e| e.kind_id()).collect();
    assert_eq!(kinds, vec!["run", "ticks", "run", "hit"]);
}

#[test]
fn every_event_field_is_read() {
    let p = parse(
        r#"{"sections": [["visual-demo", "0:00-0:10"]], "events": [
          {"kind": "run", "start": "0:01.5", "count": 4, "step": 0.085, "direction": "up-down",
           "scale": "chord", "range": [64, 80], "gain": [0.1, 0.3], "instrument": "blip",
           "riser": "reverse", "land": 2.0, "quantize": "1/8"},
          {"kind": "run", "start": 1, "count": 5, "end": 2, "direction": "down", "scale": "major"},
          {"kind": "run", "start": 3, "times": [3, 3.1, 3.35], "direction": "random", "riser": true, "quantize": "off"},
          {"kind": "ticks", "times": [4, 4.4], "sound": "clap", "notes": [70, 72], "accent": [4.4], "gain": 0.5},
          {"kind": "ticks", "start": 5, "every": [0.4, 0.4, "1/16"], "count": 6, "sound": "mallet", "contour": "wave"},
          {"kind": "hit", "at": 6, "strength": "big"},
          {"kind": "riser", "start": 7, "end": 8, "reverse": true},
          {"kind": "entrance", "at": 1, "grid": {"cols": 3, "rows": 2, "objects": 1, "shadow": true, "sky": true},
           "timing": {"ground": "clock", "move": "stepped"}, "voices": {"ground": "chip", "sky": "none"},
           "sync": "settle", "pitch": "order", "scale": "major", "range": [70, 90], "cluster": 20,
           "maxRate": 8, "land": "big", "riser": false, "fps": 24, "quantize": "1/16"},
          {"kind": "exit", "at": 8, "pieces": [{"role": "ground", "col": 0, "row": 0}], "siteTiming": {"start": 0.1}, "end": false}
        ]}"#,
    )
    .unwrap();
    let e = &p.events;
    let EventKind::Run(r) = &e[0].kind else {
        panic!()
    };
    assert_eq!(r.start, 1.5);
    assert!(matches!(r.timing, RunTiming::Step { count: 4, .. }));
    assert_eq!(r.direction, Contour::UpDown);
    assert_eq!(r.scale, ScaleKind::Chord);
    assert_eq!(r.range, (64, 80));
    assert_eq!(r.gain, Some((0.1, 0.3)));
    assert_eq!(r.instrument, Some(Instrument::Blip));
    assert_eq!(r.riser, RiserKind::Reverse);
    assert_eq!(r.land, Some(2.0));
    assert_eq!(r.quantize, Quant::Grid(0.5));
    let EventKind::Run(r) = &e[1].kind else {
        panic!()
    };
    assert!(matches!(r.timing, RunTiming::End { count: 5, end } if end == 2.0));
    assert_eq!(
        (r.direction, r.scale, r.quantize),
        (Contour::Down, ScaleKind::Major, Quant::Grid(0.25))
    );
    let EventKind::Run(r) = &e[2].kind else {
        panic!()
    };
    assert_eq!(r.timing, RunTiming::Times(vec![3.0, 3.1, 3.35]));
    assert_eq!((r.riser, r.quantize), (RiserKind::Up, Quant::Off));
    let EventKind::Ticks(t) = &e[3].kind else {
        panic!()
    };
    assert_eq!(t.sound, Instrument::Clap);
    assert_eq!(t.pitch, TickPitch::Notes(vec![70, 72]));
    assert_eq!((t.accent.clone(), t.gain), (vec![4.4], 0.5));
    let EventKind::Ticks(t) = &e[4].kind else {
        panic!()
    };
    assert!(
        matches!(&t.timing, TickTiming::Every { start, every, count: 6 } if *start == 5.0 && every.len() == 3)
    );
    assert_eq!(
        (t.sound, t.pitch.clone()),
        (Instrument::Mallet, TickPitch::Contour(Contour::Wave))
    );
    let EventKind::Hit(h) = &e[5].kind else {
        panic!()
    };
    assert_eq!((h.at, h.strength), (6.0, Strength::Big));
    let EventKind::Riser(r) = &e[6].kind else {
        panic!()
    };
    assert!(r.reverse && r.start == 7.0 && r.end == 8.0);
    let EventKind::Scene(s) = &e[7].kind else {
        panic!()
    };
    assert!(!s.exit && s.scene.approximate && s.scene.source == "grid");
    assert_eq!(s.scene.pieces.len(), 1 + 6 + 1 + 1);
    assert_eq!(s.scene.timing.ground, "clock");
    assert_eq!(s.sync, Some(Sync::Settle));
    assert_eq!(s.pitch, PitchMode::Order);
    assert_eq!(
        (s.cluster, s.max_rate, s.land, s.riser),
        (0.02, 8.0, Some(Strength::Big), RiserKind::None)
    );
    assert_eq!(
        (s.fps, s.quantize, s.range),
        (Some(24.0), Quant::Grid(0.25), (70, 90))
    );
    let EventKind::Scene(s) = &e[8].kind else {
        panic!()
    };
    assert!(
        s.exit && s.land.is_none() && s.riser == RiserKind::Reverse && s.quantize == Quant::Off
    );
    assert!(close(s.scene.timing.start, 0.1));
}

#[test]
fn bad_combinations_are_rejected_with_every_problem() {
    let errs = parse(
        r#"{"sections": [["visual-demo", "0:00-0:10"]], "events": [
          {"kind": "run", "start": 1, "count": 3, "times": [1, 2]},
          {"kind": "run", "start": 1, "count": 3, "step": "1/16", "end": 2},
          {"kind": "run", "start": 1, "step": "1/16"},
          {"kind": "run", "start": 1, "count": 3, "step": "1/7"},
          {"kind": "run", "count": 3, "step": "1/16", "wobble": 1},
          {"kind": "ticks", "times": [1], "every": 0.2},
          {"kind": "hit"},
          {"kind": "entrance", "grid": {"cols": 2, "rows": 2}, "from": "x.json"},
          {"kind": "entrance", "pieces": [{"role": "ground"}], "timing": {}, "siteTiming": {}},
          {"kind": "entrance", "pieces": [{"role": "tree"}], "timing": {"start": 1.5}},
          {"kind": "boom"}
        ], "tempo": 3}"#,
    )
    .unwrap_err();
    let all = errs.join("\n");
    for want in [
        "event 1 (run): give either times, or count with step or end, not both",
        "event 2 (run): give step or end, not both",
        "event 3 (run): step needs a count",
        "event 4 (run): step must be a note value",
        "event 5 (run): needs a start",
        "event 5 (run): unknown key \"wobble\"",
        "event 6 (ticks): give either times, or start with every and count, not both",
        "event 7 (hit): needs at",
        "event 8 (entrance): give exactly one source",
        "event 9 (entrance): give timing (ms) or siteTiming (seconds), not both",
        "role must be one of shadow, ground, object, sky",
        "entrance.start must be a whole number of ms from 0 to 60000",
        "event 11: kind must be run, ticks, hit, riser, entrance or exit",
        "plan: unknown key \"tempo\"",
    ] {
        assert!(all.contains(want), "missing {want:?} in\n{all}");
    }
}

// ------------------------------------------------------------------ runs

#[test]
fn a_sixteenth_at_150_bpm_is_a_tenth_of_a_second() {
    let p = planned(STING);
    assert!(close(p.timing.bpm, 150.0));
    let run = notes(&p, 0, "run");
    assert_eq!(run.len(), 10);
    for (k, n) in run.iter().enumerate() {
        assert!(close(n.time, 0.2 + 0.1 * k as f64), "{k}: {}", n.time);
    }
    assert!(run.last().unwrap().time < 1.2);
    let land = notes(&p, 0, "hit kick");
    assert!(close(land[0].time, 1.2));
}

#[test]
fn count_and_end_derive_the_step() {
    let p = planned(
        r#"{"bpm": 120, "sections": [["visual-demo", "0:00-0:04"]],
            "events": [{"kind": "run", "start": 1, "count": 5, "end": 2}]}"#,
    );
    let t: Vec<f64> = notes(&p, 0, "run").iter().map(|n| n.time).collect();
    assert_eq!(t, vec![1.0, 1.25, 1.5, 1.75, 2.0]);
}

#[test]
fn a_run_that_would_overshoot_its_landing_is_compressed_with_a_warning() {
    let p = planned(
        r#"{"bpm": 150, "sections": [["visual-demo", "0:00-0:04"]],
            "events": [{"kind": "run", "start": 0.2, "count": 12, "step": "1/16", "land": 1.2}]}"#,
    );
    let t: Vec<f64> = notes(&p, 0, "run").iter().map(|n| n.time).collect();
    assert_eq!(t.len(), 12);
    // The landing becomes the next step: (1.2 - 0.2) / 12.
    for (k, x) in t.iter().enumerate() {
        assert!(close(*x, 0.2 + k as f64 / 12.0), "{k}: {x}");
    }
    assert!(*t.last().unwrap() < 1.2);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("step compressed from 0.1000 s to 0.0833 s")),
        "{:?}",
        p.warnings
    );
}

fn run_midis(direction: &str, count: usize) -> Vec<i32> {
    let p = planned(&format!(
        r#"{{"bpm": 150, "feel": "chip", "sections": [["visual-demo", "0:00-0:04"]],
            "events": [{{"kind": "run", "start": 0.5, "count": {count}, "step": "1/16",
                         "direction": "{direction}", "range": [60, 76]}}]}}"#
    ));
    notes(&p, 0, "run").iter().map(|n| n.midi).collect()
}

#[test]
fn contours_order_pitches_inside_the_range() {
    // C major pentatonic from 60 to 76: 60 62 64 67 69 72 74 76.
    assert_eq!(run_midis("up", 5), vec![67, 69, 72, 74, 76]);
    assert_eq!(run_midis("down", 5), vec![76, 74, 72, 69, 67]);
    assert_eq!(run_midis("up-down", 5), vec![72, 74, 76, 74, 72]);
    assert_eq!(run_midis("down-up", 5), vec![76, 74, 72, 74, 76]);
    let r = run_midis("random", 12);
    assert!(r.iter().all(|m| (60..=76).contains(m)));
    assert!(r.windows(2).all(|w| w[0] != w[1]));
    // More notes than the pool: still rising, from the bottom to the top.
    let up = run_midis("up", 12);
    assert!(up.windows(2).all(|w| w[1] >= w[0]) && up[0] == 60 && up[11] == 76);
}

#[test]
fn wave_ticks_go_up_and_down() {
    let p = planned(
        r#"{"bpm": 150, "feel": "chip", "sections": [["visual-demo", "0:00-0:06"]],
            "events": [{"kind": "ticks", "start": 1, "every": 0.2, "count": 8, "contour": "wave"}]}"#,
    );
    let m: Vec<i32> = notes(&p, 0, "tick").iter().map(|n| n.midi).collect();
    // The high pentatonic set 84 86 88 91 93 96, as a triangle wave.
    assert_eq!(m, vec![84, 86, 88, 91, 93, 91, 88, 86]);
}

#[test]
fn gain_ramps_follow_the_direction() {
    let p = planned(STING);
    let abs = |n: &&EventNote| match n.level {
        Level::Abs(a) => a,
        Level::Rel(_) => panic!("runs are absolute"),
    };
    let up: Vec<f64> = notes(&p, 0, "run").iter().map(abs).collect();
    assert!(close(up[0], 0.09) && close(up[9], 0.2));
    assert!(up.windows(2).all(|w| w[1] > w[0]));
    let down: Vec<f64> = notes(&p, 2, "run").iter().map(abs).collect();
    assert!(close(down[0], 0.2) && close(down[8], 0.09));
    assert!(down.windows(2).all(|w| w[1] < w[0]));
}

// ------------------------------------------------------------------ ticks

#[test]
fn an_every_list_cycles() {
    let p = planned(
        r#"{"bpm": 150, "sections": [["visual-demo", "0:00-0:06"]],
            "events": [{"kind": "ticks", "start": 1, "every": [0.4, 0.4, 0.2], "count": 7}]}"#,
    );
    let t: Vec<f64> = notes(&p, 0, "tick").iter().map(|n| n.time).collect();
    let want = [1.0, 1.4, 1.8, 2.0, 2.4, 2.8, 3.0];
    assert_eq!(t.len(), want.len());
    for (a, b) in t.iter().zip(want) {
        assert!(close(*a, b), "{t:?}");
    }
}

#[test]
fn accents_are_louder_and_doubled_a_fourth_up() {
    let p = planned(STING);
    let acc = notes(&p, 1, "accent");
    assert_eq!(acc.len(), 1);
    assert!(close(acc[0].time, 3.8));
    assert_eq!(acc[0].level, Level::Rel(1.5));
    let echo = notes(&p, 1, "accent echo");
    assert_eq!(echo.len(), 1);
    assert!(close(echo[0].time, 3.85));
    assert_eq!(echo[0].midi, acc[0].midi + 5);
    assert_eq!(notes(&p, 1, "tick").len(), 10);
    assert!(notes(&p, 1, "tick")
        .iter()
        .all(|n| n.level == Level::Rel(1.0)));
    // Seeded pitches from the high pentatonic set, never the same twice running.
    let m: Vec<i32> = p
        .realized
        .notes
        .iter()
        .filter(|n| n.event == 1 && n.what != "accent echo")
        .map(|n| n.midi)
        .collect();
    assert!(m.iter().all(|x| [84, 86, 88, 91, 93, 96].contains(x)));
    assert!(m.windows(2).all(|w| w[0] != w[1]));
}

// ------------------------------------------------------------------ tempo and quantizing

#[test]
fn the_reference_ticks_choose_150_bpm() {
    let text = STING.replace("\"bpm\": 150,", "");
    let p = planned(&text);
    assert!(close(p.timing.bpm, 150.0), "{}", p.timing.bpm);
    assert_eq!(p.bpm_source, "searched with events");
}

fn hit_time(at: &str, quantize: &str) -> (f64, Vec<String>) {
    let p = planned(&format!(
        r#"{{"bpm": 150, "sections": [["visual-demo", "0:00-0:04"]],
            "events": [{{"kind": "hit", "at": {at}{quantize}}}]}}"#
    ));
    (notes(&p, 0, "hit kick")[0].time, p.warnings.clone())
}

#[test]
fn times_snap_within_15_ms_and_stay_exact_beyond() {
    let (t, w) = hit_time("1.21", "");
    assert!(close(t, 1.2) && w.is_empty(), "{t} {w:?}");
    let (t, w) = hit_time("1.186", "");
    assert!(close(t, 1.2) && w.is_empty(), "{t}");
    let (t, w) = hit_time("1.23", "");
    assert!(close(t, 1.23), "{t}");
    assert!(
        w.iter()
            .any(|w| w.contains("1 of 1 times are more than 15 ms off the 1/16 grid")),
        "{w:?}"
    );
    let (t, w) = hit_time("1.21", r#", "quantize": "off""#);
    assert!(close(t, 1.21) && w.is_empty());
    // A 1/8 grid at 150 BPM is 0.2 s.
    let (t, _) = hit_time("1.31", r#", "quantize": "1/8""#);
    assert!(close(t, 1.31));
    let (t, _) = hit_time("1.39", r#", "quantize": "1/8""#);
    assert!(close(t, 1.4));
}

// ------------------------------------------------------------------ transitions

#[test]
fn a_boundary_a_run_lands_on_gets_no_automatic_transition() {
    let p = planned(STING);
    let t = &p.arrangement.transitions;
    assert_eq!(t[0].kind, "event");
    assert!(!t[0].hit);
    // The build's own closing run is left out too.
    assert!(!p
        .arrangement
        .events
        .iter()
        .any(|e| e.section == 0 && e.role == Role::Motif));
    // Without the events the boundary has its automatic transition and hit.
    let p = parse(STING).unwrap();
    let bare = arrange_plan(
        &Request {
            specs: &p.sections,
            feel: Feel::Chip,
            seed: 7,
            instruments: Instrument::ALL.to_vec(),
            stems: false,
            gain_from: None,
        },
        &Options {
            bpm: Some(150.0),
            ..Options::default()
        },
    )
    .unwrap();
    assert_ne!(bare.arrangement.transitions[0].kind, "event");
    assert!(bare.arrangement.transitions[0].hit);
    assert!(bare
        .arrangement
        .events
        .iter()
        .any(|e| e.section == 0 && e.role == Role::Motif));
}

// ------------------------------------------------------------------ allow-lists

#[test]
fn new_roles_resolve_through_their_fallbacks() {
    use Instrument::*;
    let hb = HeavyBass;
    assert_eq!(
        candidates_in(Role::Fx, hb, Feel::Chip),
        vec![Blip, Mallet, Shaker]
    );
    assert_eq!(candidates_in(Role::Riser, hb, Feel::Spring), vec![Riser]);
    assert_eq!(
        candidates_in(Role::Backbeat, hb, Feel::Chip),
        vec![Clap, Rim]
    );
    assert_eq!(
        candidates_in(Role::Backbeat, hb, Feel::Spring),
        vec![Rim, Clap]
    );
    assert_eq!(
        candidates_in(Role::Lead, hb, Feel::Chip),
        vec![Chip, Flute, Mallet, Guitar]
    );
    assert_eq!(
        candidates_in(Role::Lead, hb, Feel::Summer),
        vec![Flute, Chip, Mallet, Guitar]
    );
    assert_eq!(
        candidates_in(Role::Picking, hb, Feel::Chip),
        vec![SynthPluck, Guitar, Mallet]
    );
    assert_eq!(
        candidates_in(Role::Picking, hb, Feel::Spring),
        vec![Guitar, SynthPluck, Mallet]
    );
}

fn tick_instruments(list: &str) -> (Vec<Option<Instrument>>, Vec<Art>) {
    let text = format!(
        r#"{{"bpm": 150, "feel": "chip", "sections": [["visual-demo", "0:00-0:04", {list}]],
            "events": [{{"kind": "ticks", "times": [1, 2]}}, {{"kind": "riser", "start": 2, "end": 3}}]}}"#
    );
    let p = planned(&text);
    let allowed: Vec<Instrument> = parse(&text).unwrap().sections[0]
        .instruments
        .clone()
        .unwrap();
    let v = event_voices(&p.realized.notes, &p.arrangement, &|_| allowed.clone());
    (
        v.iter().map(|v| v.as_ref().map(|v| v.instrument)).collect(),
        v.iter().flatten().map(|v| v.event.art).collect(),
    )
}

#[test]
fn ticks_fall_back_from_blip_to_mallet_to_shaker() {
    use Instrument::*;
    let (i, _) = tick_instruments(r#"["blip", "mallet", "riser"]"#);
    assert_eq!(i, vec![Some(Blip), Some(Blip), Some(Riser)]);
    let (i, art) = tick_instruments(r#"["mallet", "shaker"]"#);
    assert_eq!(i, vec![Some(Mallet), Some(Mallet), None]);
    // Mallet as fx plays short notes.
    assert!(art.iter().all(|a| *a == Art::Muted));
    let (i, _) = tick_instruments(r#"["shaker"]"#);
    assert_eq!(i, vec![Some(Shaker), Some(Shaker), None]);
    let (i, _) = tick_instruments(r#"["pad"]"#);
    assert_eq!(i, vec![None, None, None]);
}

#[test]
fn skipped_event_sounds_are_reported() {
    let p = parse(
        r#"{"bpm": 150, "feel": "chip", "sections": [["visual-demo", "0:00-0:03", ["pad", "kick"]]],
            "events": [{"kind": "ticks", "times": [1, 2]}]}"#,
    )
    .unwrap();
    let r = render_with(
        &Request {
            specs: &p.sections,
            feel: Feel::Chip,
            seed: 7,
            instruments: Instrument::ALL.to_vec(),
            stems: false,
            gain_from: None,
        },
        &options(&p),
    )
    .unwrap();
    let ev = &r.cue.events[0];
    assert!(ev.sounds.is_empty());
    assert_eq!(ev.skipped.len(), 2);
    assert!(r
        .cue
        .warnings
        .iter()
        .any(|w| w.contains("2 sound(s) skipped")));
}

// ------------------------------------------------------------------ keys

#[test]
fn every_key_keeps_the_bass_and_lead_in_range() {
    let plan = r#"[["visual-demo","0:00-0:08"],["reveal","0:08-0:14"],["reflect","0:14-0:24"],
                   ["recap-cta","0:24-0:32"],["outro-end-screen","0:32-0:40"]]"#;
    let p = parse(plan).unwrap();
    for feel in [Feel::Spring, Feel::Chip] {
        for name in [
            "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
        ] {
            let key = parse_key(name).unwrap();
            let pl = arrange_plan(
                &Request {
                    specs: &p.sections,
                    feel,
                    seed: 3,
                    instruments: Instrument::ALL.to_vec(),
                    stems: false,
                    gain_from: None,
                },
                &Options {
                    key: Some(key),
                    ..Options::default()
                },
            )
            .unwrap();
            let arr = &pl.arrangement;
            let shift = key_shift(feel, Some(key));
            for sp in &arr.sections {
                for c in &sp.chords {
                    let b = fold_bass(c.voicing().bass + shift);
                    assert!(
                        (BASS_LO..=BASS_HI).contains(&b),
                        "{feel:?} {name}: {c:?} bass {b}"
                    );
                }
            }
            for e in &arr.events {
                if e.role == Role::Lead {
                    assert!(
                        (72..=90).contains(&e.midi),
                        "{feel:?} {name}: lead {}",
                        e.midi
                    );
                }
                // Held bass notes are the voicing roots.
                if e.role == Role::Bass && e.art == Art::Held {
                    assert!(
                        (BASS_LO..=BASS_HI).contains(&e.midi),
                        "{feel:?} {name}: bass {}",
                        e.midi
                    );
                }
            }
            assert!(arr.events.iter().any(|e| e.role == Role::Lead));
        }
    }
    // The home keys change nothing.
    assert_eq!(key_shift(Feel::Spring, Some(parse_key("D").unwrap())), 0);
    assert_eq!(
        key_shift(
            Feel::Chip,
            Some(Key {
                tonic: 0,
                flats: false
            })
        ),
        0
    );
}

#[test]
fn keys_parse_and_reject_minor() {
    assert_eq!(parse_key("C#").unwrap().tonic, 1);
    assert_eq!(parse_key("Db").unwrap().tonic, 1);
    assert_eq!(parse_key("bb").unwrap().tonic, 10);
    assert_eq!(parse_key("G major").unwrap().tonic, 7);
    assert!(parse_key("Am").unwrap_err().contains("major keys only"));
    assert!(parse_key("H").is_err());
}

// ------------------------------------------------------------------ endings

fn render_text(text: &str, edit: impl Fn(&mut Options)) -> cozy_score::render::Rendered {
    let p = parse(text).unwrap();
    let mut o = options(&p);
    edit(&mut o);
    render_with(
        &Request {
            specs: &p.sections,
            feel: p.feel.unwrap_or(Feel::Spring),
            seed: 7,
            instruments: p.feel.unwrap_or(Feel::Spring).default_instruments(),
            stems: false,
            gain_from: None,
        },
        &o,
    )
    .unwrap()
}

#[test]
fn the_fade_out_defaults_to_a_tenth_of_the_length() {
    let r = render_text(
        r#"{"feel": "chip", "sections": [["title-sting", "0:00-0:07"]]}"#,
        |_| {},
    );
    assert!(close(r.cue.fade_out_s, 0.7), "{}", r.cue.fade_out_s);
    let r = render_text(
        r#"[["intro", "0:00-0:40"], ["outro-end-screen", "0:40-0:50"]]"#,
        |o| o.fade_out = FadeOut::Auto,
    );
    assert!(close(r.cue.fade_out_s, 3.0));
    // The original rule for untouched array plans: min(3 s, the last section).
    let r = render_text(
        r#"[["intro", "0:00-0:04"], ["outro-end-screen", "0:04-0:06"]]"#,
        |o| o.fade_out = FadeOut::Legacy,
    );
    let last = r.cue.sections.last().unwrap().start_s;
    assert!(close(r.cue.fade_out_s, 6.0 - last) && r.cue.fade_out_s < 3.0);
    let r = render_text(r#"{"sections": [["intro", "0:00-0:06"]]}"#, |o| {
        o.fade_out = FadeOut::Seconds(1.25)
    });
    assert!(close(r.cue.fade_out_s, 1.25));
}

#[test]
fn a_tail_makes_the_audio_longer_than_the_video() {
    let text = r#"{"feel": "chip", "bpm": 150, "sections": [["title-sting", "0:00-0:03.2"]],
                   "events": [{"kind": "hit", "at": 3.0, "strength": "big"}]}"#;
    let plain = render_text(text, |_| {});
    let tail = render_text(text, |o| o.tail = 1.5);
    assert_eq!(plain.full.len(), (3.2 * 44100.0f64).round() as usize);
    assert_eq!(tail.full.len(), (4.7 * 44100.0f64).round() as usize);
    assert!(close(tail.cue.video_duration_s, 3.2) && close(tail.cue.audio_duration_s, 4.7));
    // Same level: up to where the plain render's fade begins, the 16-bit
    // samples match (the reverb's block sums differ only in the last bits).
    let fade_start = ((3.2 - plain.cue.fade_out_s) * 44100.0).floor() as usize - 1;
    let pcm = |x: &[f64]| -> Vec<i32> { x.iter().map(|v| (v * 32767.0).round() as i32).collect() };
    assert_eq!(
        pcm(&plain.full.l[..fade_start]),
        pcm(&tail.full.l[..fade_start])
    );
    assert_eq!(
        pcm(&plain.full.r[..fade_start]),
        pcm(&tail.full.r[..fade_start])
    );
    assert_eq!(plain.cue.master.gain, tail.cue.master.gain);
}

// ------------------------------------------------------------------ determinism

#[test]
fn events_never_shift_other_randomness() {
    let base = r#"{"feel": "chip", "bpm": 150, "sections": [["visual-demo", "0:00-0:06"], ["outro-end-screen", "0:06-0:08"]], "events": [EVENTS]}"#;
    let ticks = r#"{"kind": "ticks", "times": [1, 1.4, 2.2, 3.0]}"#;
    let random_run =
        r#"{"kind": "run", "start": 4, "count": 6, "step": "1/16", "direction": "random"}"#;
    let one = planned(&base.replace("EVENTS", ticks));
    let two = planned(&base.replace("EVENTS", &format!("{ticks}, {random_run}")));
    let none = planned(&base.replace("EVENTS", ""));
    // The arrangement is untouched by events that replace no transition.
    assert_eq!(one.arrangement.events, none.arrangement.events);
    assert_eq!(two.arrangement.events, none.arrangement.events);
    // Adding an event leaves the others' notes (and seeded pitches) alone.
    let first = |p: &Planned| -> Vec<EventNote> {
        p.realized
            .notes
            .iter()
            .filter(|n| n.event == 0)
            .cloned()
            .collect()
    };
    assert_eq!(first(&one), first(&two));
}

#[test]
fn the_backbeat_falls_back_between_clap_and_rim() {
    let p = parse(
        r#"[["visual-demo", "0:00-0:08", ["rim", "kick"]], ["explain", "0:08-0:16", ["clap"]]]"#,
    )
    .unwrap();
    for (feel, first, second) in [
        (Feel::Chip, Instrument::Rim, Instrument::Clap),
        (Feel::Spring, Instrument::Rim, Instrument::Clap),
    ] {
        let pl = arrange_plan(
            &Request {
                specs: &p.sections,
                feel,
                seed: 7,
                instruments: Instrument::ALL.to_vec(),
                stems: false,
                gain_from: None,
            },
            &Options::default(),
        )
        .unwrap();
        let own: Vec<Option<Vec<Instrument>>> =
            p.sections.iter().map(|s| s.instruments.clone()).collect();
        let res = cozy_score::resolve::resolve_sections(&pl.arrangement, &Instrument::ALL, &own);
        assert_eq!(res[0].binding(Role::Backbeat), Some(first), "{feel:?}");
        assert_eq!(res[1].binding(Role::Backbeat), Some(second), "{feel:?}");
        let all =
            cozy_score::resolve::resolve_sections(&pl.arrangement, &Instrument::ALL, &[None, None]);
        let want = if feel == Feel::Chip {
            Instrument::Clap
        } else {
            Instrument::Rim
        };
        assert_eq!(all[0].binding(Role::Backbeat), Some(want));
    }
}
