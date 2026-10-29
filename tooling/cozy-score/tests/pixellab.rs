//! The pixellab port: the entrance schedule matches pixellab's own parity
//! fixture (copied from jlawcordova-lab at 8a3179f), and every way of giving
//! a scene describes the same schedule.

use cozy_score::pixellab::{
    schedule, settled_by, timing_from_entrance, timing_from_site, Piece, Timing, GROUND_ORDERS,
    OBJECT_ORDERS,
};
use serde_json::{json, Value};

fn parity() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/pixellab/parity.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The fixture's pieces, moved onto pixellab's grid. The site's pixels are on
/// 32x16 tiles and pixellab's on 44x22, so x and y are scaled by 44/32, as
/// pixellab's own parity test does.
fn pieces(list: &Value) -> Vec<Piece> {
    let n = |p: &Value, k: &str| p.get(k).and_then(Value::as_f64);
    let scale = 44.0 / 32.0;
    list.as_array()
        .unwrap()
        .iter()
        .map(|p| Piece {
            enter: p["enter"].as_str().unwrap_or("").to_string(),
            col: n(p, "col"),
            row: n(p, "row"),
            x: n(p, "x").map(|v| v * scale),
            y: n(p, "y").map(|v| v * scale),
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

#[test]
fn fixture_keeps_its_note_and_site() {
    let p = parity();
    assert!(p["note"].as_str().unwrap().contains("do not edit"));
    assert_eq!(p["site"]["file"], "src/lib/island-entrance.mjs");
}

#[test]
fn schedule_matches_pixellab_for_every_case() {
    let p = parity();
    let cases = p["cases"].as_array().unwrap();
    assert!(cases.len() >= 40);
    let mut grounds = Vec::new();
    let mut objects = Vec::new();
    let mut seeds = Vec::new();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let list = pieces(&p["pieces"][case["pieces"].as_str().unwrap()]);
        // The site's seconds, read as `--timing` reads them.
        let timing = timing_from_site(&case["timing"]).unwrap();
        grounds.push(timing.ground.clone());
        if !timing.with {
            objects.push(timing.objects.clone());
        }
        seeds.push(timing.seed as i64);
        let plan = schedule(&list, &timing).entrances;
        let delays = case["delays"].as_array().unwrap();
        assert_eq!(plan.len(), delays.len(), "{name}");
        for (i, (ours, theirs)) in plan.iter().zip(delays).enumerate() {
            match (ours, theirs.as_f64()) {
                (None, None) => {}
                (Some(e), Some(d)) => {
                    assert!(close(e.delay, d), "{name}: piece {i}: {} vs {d}", e.delay);
                    let kind = &case["kinds"][&list[i].enter];
                    assert_eq!(e.mv, kind["move"].as_str().unwrap(), "{name}");
                    assert!(
                        close(e.duration, kind["duration"].as_f64().unwrap()),
                        "{name}"
                    );
                    if let Some(times) = kind.get("times") {
                        assert!(close(e.peak, times[1].as_f64().unwrap()), "{name}: peak");
                    }
                }
                (o, t) => panic!("{name}: piece {i}: ours {o:?}, pixellab's {t:?}"),
            }
        }
        let settled = case["settledBy"].as_f64().unwrap();
        assert!(
            close(settled_by(&plan), settled),
            "{name}: {} vs {settled}",
            settled_by(&plan)
        );
    }
    for g in GROUND_ORDERS {
        assert!(grounds.iter().any(|x| x == g), "no case for ground {g}");
    }
    for o in OBJECT_ORDERS {
        assert!(objects.iter().any(|x| x == o), "no case for objects {o}");
    }
    assert!(seeds.contains(&1) && seeds.contains(&7));
}

#[test]
fn default_timing_is_the_hero_case() {
    let p = parity();
    let case = &p["cases"][0];
    assert_eq!(case["name"], "hero TIMING");
    assert_eq!(
        Timing::of(&json!({})),
        timing_from_site(&case["timing"]).unwrap()
    );
}

#[test]
fn rejects_unknown_fields_and_bad_values_in_pixellabs_words() {
    let e =
        timing_from_entrance(&json!({"start": 12.5, "ground": "spiral", "wobble": 1, "peak": 2}))
            .unwrap_err();
    let all = e.join("\n");
    assert!(
        all.contains("entrance.start must be a whole number of ms from 0 to 60000"),
        "{all}"
    );
    assert!(
        all.contains("entrance.ground must be one of rows, cols"),
        "{all}"
    );
    assert!(all.contains("entrance: unknown property 'wobble'"), "{all}");
    assert!(
        all.contains("entrance.peak must be a number from 0 to 1"),
        "{all}"
    );
    let e = timing_from_entrance(&json!({"start": 70000})).unwrap_err();
    assert!(e[0].contains("from 0 to 60000"));
    let e = timing_from_site(&json!({"wobble": 0.2})).unwrap_err();
    assert_eq!(e, vec!["unknown timing field 'wobble'".to_string()]);
    let e = timing_from_site(&json!({"start": "soon"})).unwrap_err();
    assert_eq!(
        e,
        vec!["timing.start must be a number of seconds".to_string()]
    );
}

// ------------------------------------------------------------------ sources

mod scoring {
    use cozy_score::arrange::Art;
    use cozy_score::events::{EventKind, EventNote, ParseContext, SceneSpec};
    use cozy_score::pixellab::{
        draft_from_sequence, num, spans, Scene, Sequence, SequenceDraft, FIELDS,
    };
    use cozy_score::plan::parse_plan_with;
    use cozy_score::render::{arrange_plan, Options, Planned, Request};
    use cozy_score::types::{Feel, Instrument, Role};
    use serde_json::{json, Map, Value};
    use std::path::Path;

    fn tmp() -> &'static Path {
        Path::new(env!("CARGO_TARGET_TMPDIR"))
    }

    fn plan_with(event: Value, base: &Path) -> (cozy_score::plan::Plan, Planned) {
        let text = json!({
            "feel": "chip", "bpm": 150,
            "sections": [["visual-demo", "0:00-0:10"]],
            "events": [event]
        })
        .to_string();
        let p = parse_plan_with(
            &text,
            &ParseContext {
                base,
                document: None,
                seed: 7,
            },
        )
        .unwrap();
        let planned = arrange_plan(
            &Request {
                specs: &p.sections,
                feel: Feel::Chip,
                seed: 7,
                instruments: Instrument::ALL.to_vec(),
                stems: false,
                gain_from: None,
            },
            &Options {
                events: p.events.clone(),
                bpm: p.bpm,
                fps: p.fps,
                ..Options::default()
            },
        )
        .unwrap();
        (p, planned)
    }

    fn scene_of(p: &cozy_score::plan::Plan) -> &SceneSpec {
        match &p.events[0].kind {
            EventKind::Scene(s) => s,
            _ => panic!("not a scene"),
        }
    }

    fn pieces_json(sc: &Scene) -> Value {
        Value::Array(
            sc.pieces
                .iter()
                .map(|p| {
                    let mut o = Map::new();
                    o.insert("role".into(), json!(p.enter));
                    for (k, v) in [("col", p.col), ("row", p.row), ("x", p.x), ("y", p.y)] {
                        if let Some(v) = v {
                            o.insert(k.into(), num(v));
                        }
                    }
                    Value::Object(o)
                })
                .collect(),
        )
    }

    /// What `pixellab scene entrance show --json` prints for a scene (times
    /// rounded to whole ms, as pixellab does).
    fn show_json(sc: &Scene, entrance: &Value) -> Value {
        let ms = |s: f64| num((s * 1000.0).round());
        let mut timing = Map::new();
        for (k, _, d) in FIELDS {
            timing.insert(
                k.into(),
                entrance.get(k).cloned().unwrap_or_else(|| d.to_value()),
            );
        }
        let pieces: Vec<Value> = sc
            .pieces
            .iter()
            .zip(&sc.entrances)
            .zip(pieces_json(sc).as_array().unwrap())
            .enumerate()
            .map(|(i, ((_, e), pj))| {
                let mut o = pj.as_object().unwrap().clone();
                let e = e.as_ref().unwrap();
                o.insert("path".into(), json!(i.to_string()));
                o.insert("delay".into(), ms(e.delay));
                o.insert("duration".into(), ms(e.duration));
                o.insert("move".into(), json!(e.mv));
                Value::Object(o)
            })
            .collect();
        json!({"scene": "grid-scene", "entrance": entrance, "timing": timing,
               "settledBy": ms(sc.settled_by), "pieces": pieces})
    }

    #[test]
    fn every_source_gives_the_same_schedule() {
        let grid = json!({"cols": 5, "rows": 4, "objects": 4, "shadow": true, "sky": true});
        let (pa, _) = plan_with(
            json!({"kind": "entrance", "grid": grid, "timing": {"ground": "center", "start": 300}}),
            tmp(),
        );
        let a = &scene_of(&pa).scene;
        assert!(a.approximate && a.source == "grid");
        assert_eq!(a.pieces.len(), 1 + 20 + 4 + 1);
        // Block (u, v) at x = (u - v) 22, y = (u + v) 11.
        let b5 = &a.pieces[1 + 7];
        assert_eq!(
            (b5.col, b5.row, b5.x, b5.y),
            (Some(2.0), Some(1.0), Some(22.0), Some(33.0))
        );
        // Objects stand on distinct cells.
        let mut cells: Vec<(i64, i64)> = a
            .pieces
            .iter()
            .filter(|p| p.enter == "object")
            .map(|p| (p.x.unwrap() as i64, p.y.unwrap() as i64))
            .collect();
        cells.sort();
        cells.dedup();
        assert_eq!(cells.len(), 4);

        // The same pieces given as `pieces` + `timing` (ms) ...
        let (pb, _) = plan_with(
            json!({"kind": "entrance", "pieces": pieces_json(a), "timing": {"ground": "center", "start": 300}}),
            tmp(),
        );
        let b = &scene_of(&pb).scene;
        assert_eq!(a.entrances, b.entrances);
        assert!(!b.approximate && b.source == "pieces");
        // ... or with the site's seconds ...
        let (pc, _) = plan_with(
            json!({"kind": "entrance", "pieces": pieces_json(a), "siteTiming": {"ground": "center", "start": 0.3}}),
            tmp(),
        );
        assert_eq!(a.entrances, scene_of(&pc).scene.entrances);
        // ... or as pixellab's `scene entrance show --json` output.
        let file = tmp().join("grid-scene.json");
        std::fs::write(
            &file,
            show_json(a, &json!({"ground": "center", "start": 300})).to_string(),
        )
        .unwrap();
        let (pd, _) = plan_with(
            json!({"kind": "entrance", "from": "grid-scene.json"}),
            tmp(),
        );
        let d = &scene_of(&pd).scene;
        assert_eq!(d.source, "from");
        assert_eq!(d.name.as_deref(), Some("grid-scene"));
        assert!((d.settled_by - a.settled_by).abs() <= 0.0005);
        for (x, y) in a.entrances.iter().zip(&d.entrances) {
            let (x, y) = (x.as_ref().unwrap(), y.as_ref().unwrap());
            assert_eq!(x.mv, y.mv);
            assert!(
                (x.delay - y.delay).abs() <= 0.0005 && (x.duration - y.duration).abs() <= 0.0005
            );
            assert!((x.peak - y.peak).abs() < 1e-12);
        }
        assert_eq!(a.order, d.order);
    }

    #[test]
    fn sources_are_checked_in_pixellabs_words() {
        let text = json!({"sections": [["visual-demo", "0:00-0:05"]], "events": [
            {"kind": "entrance", "pieces": [{"role": "ground", "col": 0, "row": 0}], "timing": {"lag": -1, "objectMove": "bounce"}},
            {"kind": "entrance", "pieces": [{"role": "ground"}], "siteTiming": {"grid": 1}},
            {"kind": "entrance", "from": "missing.json"},
            {"kind": "entrance", "scene": "hero-island"}
        ]})
        .to_string();
        let errs = parse_plan_with(
            &text,
            &ParseContext {
                base: tmp(),
                document: None,
                seed: 1,
            },
        )
        .unwrap_err();
        let all = errs.join("\n");
        assert!(
            all.contains("entrance.lag must be a whole number of ms from 0 to 60000"),
            "{all}"
        );
        assert!(
            all.contains(
                "entrance.objectMove must be one of same, float, drop, pop, stepped, fade"
            ),
            "{all}"
        );
        assert!(all.contains("unknown timing field 'grid'"), "{all}");
        assert!(all.contains("cannot read"), "{all}");
        assert!(all.contains("needs --pixellab-document"), "{all}");
        // Without pixellab on PATH, a scene source says to pass `from`.
        let doc = tmp().join("none.pixellab");
        let errs = parse_plan_with(
            &text,
            &ParseContext {
                base: tmp(),
                document: Some(&doc),
                seed: 1,
            },
        )
        .unwrap_err();
        if std::process::Command::new("pixellab")
            .arg("--version")
            .output()
            .is_err()
        {
            assert!(errs.join("\n").contains("pass \"from\""), "{errs:?}");
        }
    }

    // ------------------------------------------------------------------ sound mapping

    fn row_scene(timing: Value, extra: Value) -> (cozy_score::plan::Plan, Planned) {
        let mut ev = json!({"kind": "entrance", "at": 1.0,
            "grid": {"cols": 6, "rows": 1, "objects": 0, "shadow": false},
            "timing": timing, "land": false, "riser": false});
        for (k, v) in extra.as_object().unwrap() {
            ev[k] = v.clone();
        }
        if ev["kind"] == "exit" {
            // An exit's closing hit is its `end`.
            let land = ev.as_object_mut().unwrap().remove("land").unwrap();
            ev["end"] = land;
        }
        plan_with(ev, tmp())
    }

    fn piece_notes<'a>(p: &'a Planned, what: &str) -> Vec<&'a EventNote> {
        p.realized.notes.iter().filter(|n| n.what == what).collect()
    }

    #[test]
    fn each_move_sounds_at_its_sync_point() {
        let spread = json!({"cascade": 0, "groundSpan": 1000});
        for (mv, impact, art) in [
            ("float", false, Art::Glide),
            ("drop", true, Art::Normal),
            ("pop", true, Art::Muted),
            ("stepped", false, Art::Muted),
            ("fade", false, Art::Swell),
        ] {
            let mut t = spread.clone();
            t["move"] = json!(mv);
            let (p, pl) = row_scene(t, json!({}));
            let sc = &scene_of(&p).scene;
            let notes = piece_notes(&pl, "piece ground");
            assert_eq!(notes.len(), 6, "{mv}");
            for n in &notes {
                let e = sc.entrances[n.piece.unwrap()].as_ref().unwrap();
                let sync = e.delay + if impact { e.peak * e.duration } else { 0.0 };
                assert!(
                    (n.time - (1.0 + sync)).abs() < 1e-9,
                    "{mv}: {} vs {}",
                    n.time,
                    1.0 + sync
                );
                assert_eq!(n.art, art, "{mv}");
            }
            let thuds = piece_notes(&pl, "thud");
            assert_eq!(thuds.len(), if mv == "drop" { 6 } else { 0 });
            assert!(thuds.iter().all(|t| t.role == Role::Kick));
            // A stepped move repeats min(rise, 3) times, quieter, across the move.
            assert_eq!(
                piece_notes(&pl, "step").len(),
                if mv == "stepped" { 18 } else { 0 }
            );
        }
        // `sync` forces one sync point for every piece.
        let (p, pl) = row_scene(
            json!({"cascade": 0, "move": "drop"}),
            json!({"sync": "settle"}),
        );
        let sc = &scene_of(&p).scene;
        for n in piece_notes(&pl, "piece ground") {
            let e = sc.entrances[n.piece.unwrap()].as_ref().unwrap();
            assert!((n.time - (1.0 + e.delay + e.duration)).abs() < 1e-9);
        }
    }

    #[test]
    fn an_exit_mirrors_every_sync_time() {
        let t = json!({"cascade": 0, "move": "drop"});
        let (p, entr) = row_scene(t.clone(), json!({}));
        let (_, exit) = row_scene(t, json!({"kind": "exit", "at": 4.0}));
        let settled = scene_of(&p).scene.settled_by;
        let en: Vec<f64> = piece_notes(&entr, "piece ground")
            .iter()
            .map(|n| n.time - 1.0)
            .collect();
        let mut ex: Vec<f64> = piece_notes(&exit, "piece ground")
            .iter()
            .map(|n| n.time)
            .collect();
        ex.sort_by(f64::total_cmp);
        let mut want: Vec<f64> = en.iter().map(|t| 4.0 + settled - t).collect();
        want.sort_by(f64::total_cmp);
        assert_eq!(ex.len(), want.len());
        for (a, b) in ex.iter().zip(&want) {
            assert!((a - b).abs() < 1e-9);
        }
        // Rising on the way in, falling on the way out.
        let rise: Vec<i32> = piece_notes(&entr, "piece ground")
            .iter()
            .map(|n| n.midi)
            .collect();
        let fall: Vec<i32> = piece_notes(&exit, "piece ground")
            .iter()
            .map(|n| n.midi)
            .collect();
        assert!(rise.windows(2).all(|w| w[1] >= w[0]) && rise.first() < rise.last());
        assert!(fall.windows(2).all(|w| w[1] <= w[0]) && fall.first() > fall.last());
    }

    #[test]
    fn dense_scenes_merge_into_clusters_and_keep_a_rate() {
        // 30 blocks over 0.3 s: 10 ms apart.
        let grid = json!({"cols": 30, "rows": 1, "objects": 0, "shadow": false});
        let ev = |extra: Value| {
            let mut e = json!({"kind": "entrance", "at": 0.5, "grid": grid,
                "timing": {"cascade": 0, "groundSpan": 290}, "land": false, "riser": false});
            for (k, v) in extra.as_object().unwrap() {
                e[k] = v.clone();
            }
            e
        };
        let (_, pl) = plan_with(ev(json!({})), tmp());
        let s = pl.realized.summaries[0].scene.clone().unwrap();
        assert_eq!(s.sync_points, 30);
        // Within 35 ms of a cluster's first point: 4 points a cluster.
        assert_eq!(s.clusters, 8);
        let notes = piece_notes(&pl, "piece ground");
        assert_eq!(notes.len(), s.notes_kept);
        // At most 14 a second.
        assert!(notes
            .windows(2)
            .all(|w| w[1].time - w[0].time >= 1.0 / 14.0 - 1e-9));
        assert!(s.notes_kept < s.clusters);
        // A cluster of 4: base (1 + 0.25 ln 4), base 0.11 (float: x 0.7).
        let g = match notes[0].level {
            cozy_score::events::Level::Abs(a) => a,
            _ => panic!(),
        };
        assert!(
            (g - 0.11 * (1.0 + 0.25 * libm::log(4.0)) * 0.7).abs() < 1e-12,
            "{g}"
        );
        // No merging and no cap: a note per piece.
        let (_, pl) = plan_with(ev(json!({"cluster": 0, "maxRate": 200})), tmp());
        let s = pl.realized.summaries[0].scene.clone().unwrap();
        assert_eq!((s.clusters, s.notes_kept), (30, 30));
        // Large clusters stop getting louder at 1.6 x the base.
        let (_, pl) = plan_with(ev(json!({"cluster": 1000})), tmp());
        let n = piece_notes(&pl, "piece ground");
        assert_eq!(n.len(), 1);
        let g = match n[0].level {
            cozy_score::events::Level::Abs(a) => a,
            _ => panic!(),
        };
        assert!((g - 0.11 * 1.6 * 0.7).abs() < 1e-12);
    }

    #[test]
    fn together_becomes_one_chord() {
        let (_, pl) = row_scene(json!({"ground": "together", "cascade": 0}), json!({}));
        assert!(piece_notes(&pl, "piece ground").is_empty());
        let chord = piece_notes(&pl, "piece chord");
        assert!(chord.len() >= 3);
        assert!(chord.iter().all(|n| (n.time - chord[0].time).abs() < 1e-12));
    }

    fn grid_notes(timing: Value, pitch: &str) -> (Scene, Vec<EventNote>) {
        let (p, pl) = plan_with(
            json!({"kind": "entrance", "at": 0.0,
                "grid": {"cols": 6, "rows": 6, "objects": 0, "shadow": false},
                "timing": timing, "pitch": pitch, "cluster": 0, "maxRate": 200,
                "land": false, "riser": false}),
            tmp(),
        );
        let sc = scene_of(&p).scene.clone();
        let notes = pl
            .realized
            .notes
            .iter()
            .filter(|n| n.what == "piece ground")
            .cloned()
            .collect();
        (sc, notes)
    }

    /// Whether the notes' pitches never fall as `key` rises.
    fn follows(notes: &[EventNote], key: impl Fn(usize) -> f64) -> bool {
        let mut v: Vec<(f64, i32)> = notes
            .iter()
            .map(|n| (key(n.piece.unwrap()), n.midi))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        v.windows(2).all(|w| w[1].1 >= w[0].1)
    }

    #[test]
    fn pitch_modes_order_the_notes() {
        let t = json!({"ground": "random", "groundSpan": 2000});
        let (sc, n) = grid_notes(t.clone(), "x");
        assert!(follows(&n, |i| sc.pieces[i].x.unwrap()));
        let (sc, n) = grid_notes(t.clone(), "y");
        assert!(follows(&n, |i| sc.pieces[i].y.unwrap()));
        let (sc, n) = grid_notes(t.clone(), "time");
        assert!(follows(&n, |i| sc.entrances[i].as_ref().unwrap().delay));
        // `order`: the order's own score, so `center` sounds centre-out.
        let (sc, n) = grid_notes(json!({"ground": "center"}), "order");
        assert!(follows(&n, |i| sc.order[i].unwrap()));
        let lo = n.iter().min_by_key(|x| x.midi).unwrap();
        let hi = n.iter().max_by_key(|x| x.midi).unwrap();
        assert!(sc.order[lo.piece.unwrap()].unwrap() < sc.order[hi.piece.unwrap()].unwrap());
        let (_, a) = grid_notes(t.clone(), "random");
        let (_, b) = grid_notes(t, "random");
        assert_eq!(a, b);
        assert!(a.iter().all(|x| (60..=88).contains(&x.midi)));
    }

    #[test]
    fn a_checker_ground_alternates_two_chord_tones() {
        let (sc, n) = grid_notes(json!({"ground": "checker", "groundSpan": 1000}), "time");
        let mut tones: Vec<i32> = n.iter().map(|x| x.midi).collect();
        tones.sort();
        tones.dedup();
        assert_eq!(tones.len(), 2, "{tones:?}");
        // The two groups are the two checker colours.
        let of = |g: i64| -> Vec<i32> {
            let mut v: Vec<i32> = n
                .iter()
                .filter(|x| {
                    let p = &sc.pieces[x.piece.unwrap()];
                    (p.col.unwrap() + p.row.unwrap()) as i64 % 2 == g
                })
                .map(|x| x.midi)
                .collect();
            v.dedup();
            v
        };
        assert_eq!(of(0).len(), 1);
        assert_eq!(of(1).len(), 1);
        assert_ne!(of(0), of(1));
    }

    #[test]
    fn frames_round_sounds_up_to_the_next_frame() {
        let (_, pl) = row_scene(json!({"cascade": 37}), json!({"fps": 30, "land": "land"}));
        let notes = &pl.realized.notes;
        assert!(!notes.is_empty());
        let mut moved = 0;
        for n in notes {
            let f = n.time * 30.0;
            assert!((f - f.round()).abs() < 1e-6, "{} is not on a frame", n.time);
            if let Some(s) = n.sync_time {
                moved += 1;
                assert!(
                    n.time >= s - 1e-9 && n.time < s + 1.0 / 30.0,
                    "{s} -> {}",
                    n.time
                );
            }
        }
        assert!(moved > 0);
    }

    // ------------------------------------------------------------------ sequences

    fn write_scene(dir: &Path, name: &str, cols: usize) -> f64 {
        let (p, _) = plan_with(
            json!({"kind": "entrance", "grid": {"cols": cols, "rows": 2, "objects": 1}, "timing": {}}),
            dir,
        );
        let sc = scene_of(&p).scene.clone();
        let v = show_json(&sc, &json!({}));
        std::fs::write(dir.join(format!("{name}.json")), v.to_string()).unwrap();
        v["settledBy"].as_f64().unwrap()
    }

    fn sequence_json() -> Value {
        let mut seq = json!({
            "name": "tour",
            "shots": [
                {"shot": 1, "scene": "meadow", "length": 3000, "from": 0, "entrance": true},
                {"shot": 2, "scene": "b", "length": 2500, "from": 0, "hold": 500},
                {"shot": 3, "scene": "c", "length": 3000, "from": 0},
                {"shot": 4, "scene": "d", "length": 2000, "from": 0},
                {"shot": 5, "scene": "e", "length": 2500, "from": 0},
                {"shot": 6, "scene": "harbor", "length": 4000, "from": 0, "hold": 200},
                {"shot": 7, "scene": "town", "length": 3000, "from": 0}
            ],
            "transitions": [
                {"join": 1, "kind": "cut"},
                {"join": 2, "kind": "fade", "length": 400},
                {"join": 3, "kind": "dip", "length": 600, "color": "black"},
                {"join": 4, "kind": "slide", "length": 400, "direction": "right"},
                {"join": 5, "kind": "wipe", "length": 500, "direction": "up"},
                {"join": 6, "kind": "blocks", "gap": 300}
            ]
        });
        let sp = spans(&Sequence::of(&seq));
        for (s, span) in seq["shots"].as_array_mut().unwrap().iter_mut().zip(&sp) {
            s["start"] = num(span.start);
            s["end"] = num(span.end);
        }
        seq["shots"][6]["entrance"] = json!(true);
        seq["length"] = num(sp.last().unwrap().end);
        seq
    }

    #[test]
    fn a_sequence_drafts_a_plan_that_matches_spans() {
        let dir = tmp().join("seq-entrances");
        std::fs::create_dir_all(&dir).unwrap();
        let settled_meadow = write_scene(&dir, "meadow", 4);
        let settled_harbor = write_scene(&dir, "harbor", 5);
        write_scene(&dir, "town", 3);
        let seq = sequence_json();
        let sp = spans(&Sequence::of(&seq));
        let plan_dir = tmp().join("seq-plan");
        std::fs::create_dir_all(&plan_dir).unwrap();
        let plan = draft_from_sequence(&SequenceDraft {
            sequence: &seq,
            entrances: &dir,
            types: &[(1, "explain".into())],
            plan_dir: &plan_dir,
            feel: Some("chip"),
        })
        .unwrap();
        let secs = |ms: f64| ms / 1000.0;
        let sections = plan["sections"].as_array().unwrap();
        let bounds: Vec<(String, f64, f64)> = sections
            .iter()
            .map(|s| {
                (
                    s[0].as_str().unwrap().to_string(),
                    s[1][0].as_f64().unwrap(),
                    s[1][1].as_f64().unwrap(),
                )
            })
            .collect();
        let kinds: Vec<&str> = bounds.iter().map(|b| b.0.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "intro",
                "explain",
                "visual-demo",
                "visual-demo",
                "visual-demo",
                "visual-demo",
                "outro-end-screen"
            ]
        );
        // Contiguous, from 0 to the sequence's end.
        assert_eq!(bounds[0].1, 0.0);
        assert!(bounds.windows(2).all(|w| w[0].2 == w[1].1));
        assert_eq!(bounds[6].2, secs(sp[6].end));
        // cut: at the join; fade, dip, slide, wipe: the overlap's midpoint;
        // blocks: the gap goes to the following shot.
        assert_eq!(bounds[1].1, secs(sp[1].start));
        for i in 2..=5 {
            assert!(
                (bounds[i].1 - secs((sp[i].start + sp[i - 1].end) / 2.0)).abs() < 1e-12,
                "section {i}"
            );
        }
        assert_eq!(bounds[6].1, secs(sp[5].end));
        assert_eq!(sp[6].start, sp[5].end + 300.0);
        // Events.
        let ev = plan["events"].as_array().unwrap();
        let find = |kind: &str| ev.iter().filter(|e| e["kind"] == kind).collect::<Vec<_>>();
        let entr = find("entrance");
        assert_eq!(entr.len(), 2);
        assert_eq!(entr[0]["at"], 0);
        assert_eq!(entr[1]["at"].as_f64().unwrap(), secs(sp[6].start));
        assert_eq!(entr[0]["from"], "../seq-entrances/meadow.json");
        let exits = find("exit");
        assert_eq!(exits.len(), 1);
        let shot = &seq["shots"][5];
        let want = sp[5].start + shot["length"].as_f64().unwrap() + shot["hold"].as_f64().unwrap()
            - settled_harbor;
        assert!((exits[0]["at"].as_f64().unwrap() - secs(want)).abs() < 1e-12);
        assert!(settled_meadow > 0.0);
        let hits = find("hit");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["strength"], "soft");
        assert_eq!(hits[0]["at"].as_f64().unwrap(), secs(sp[1].start));
        let risers = find("riser");
        assert_eq!(risers.len(), 2);
        assert_eq!(risers[0]["start"].as_f64().unwrap(), secs(sp[2].start));
        assert_eq!(risers[0]["end"].as_f64().unwrap(), secs(sp[1].end));
        let runs = find("run");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0]["direction"], "down"); // slide right
        assert_eq!(runs[1]["direction"], "up"); // wipe up
        assert_eq!(runs[1]["start"].as_f64().unwrap(), secs(sp[5].start));
        assert_eq!(runs[1]["end"].as_f64().unwrap(), secs(sp[4].end));
        assert!(plan["notes"].as_array().unwrap().len() > 10);
        // The draft is a plan cozy-score reads, `from` paths and all.
        let text = plan.to_string();
        parse_plan_with(
            &text,
            &ParseContext {
                base: &plan_dir,
                document: None,
                seed: 7,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_sequence_whose_times_disagree_with_spans_is_rejected() {
        let dir = tmp().join("seq-entrances-2");
        std::fs::create_dir_all(&dir).unwrap();
        for s in ["meadow", "harbor", "town"] {
            write_scene(&dir, s, 3);
        }
        let mut seq = sequence_json();
        seq["shots"][3]["start"] = json!(1234);
        let e = draft_from_sequence(&SequenceDraft {
            sequence: &seq,
            entrances: &dir,
            types: &[],
            plan_dir: tmp(),
            feel: None,
        })
        .unwrap_err();
        assert!(e[0].contains("shot 4") && e[0].contains("spans()"), "{e:?}");
        // A missing entrance file says how to export it.
        std::fs::remove_file(dir.join("town.json")).unwrap();
        let seq = sequence_json();
        let e = draft_from_sequence(&SequenceDraft {
            sequence: &seq,
            entrances: &dir,
            types: &[],
            plan_dir: tmp(),
            feel: None,
        })
        .unwrap_err();
        assert!(
            e.join("\n")
                .contains("pixellab scene entrance show town --json"),
            "{e:?}"
        );
    }
}
