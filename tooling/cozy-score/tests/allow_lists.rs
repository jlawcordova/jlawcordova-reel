//! Arrangement and allow-list behaviour, checked on note data (no audio).

use cozy_score::arrange::{arrange, Arrangement};
use cozy_score::plan::parse_plan;
use cozy_score::resolve::{resolve_sections, voices, Bus};
use cozy_score::timing::{search_tempo, snap};
use cozy_score::types::{Feel, Instrument, Role, VideoType};

use Instrument::*;

const REFERENCE: &str = include_str!("fixtures/reference-60s.json");

fn arranged(plan: &str, feel: Feel, seed: u64) -> (Arrangement, Vec<Option<Vec<Instrument>>>) {
    let specs = parse_plan(plan).unwrap();
    let ends: Vec<f64> = specs.iter().map(|s| s.end).collect();
    let t = snap(&ends, search_tempo(&ends, feel).unwrap());
    let kinds: Vec<VideoType> = specs.iter().map(|s| s.kind).collect();
    let own = specs.iter().map(|s| s.instruments.clone()).collect();
    (arrange(&kinds, &t, feel, seed), own)
}

fn lead_notes(arr: &Arrangement) -> Vec<(u64, i32, i64)> {
    arr.events
        .iter()
        .filter(|e| e.role == Role::Lead)
        .map(|e| (e.id, e.midi, (e.time * 1e6) as i64))
        .collect()
}

#[test]
fn section_list_replaces_score_list() {
    let plan = r#"[["visual-demo","0:00-0:10"],["explain","0:10-0:20",["pad","heavy-bass"]]]"#;
    let (arr, own) = arranged(plan, Feel::Spring, 7);
    let res = resolve_sections(&arr, &[Guitar, Flute, Kick], &own);
    assert_eq!(res[0].list_source, "score");
    assert_eq!(res[1].list_source, "section");
    // Not combined: explain cannot use the score's guitar or kick.
    assert_eq!(res[1].binding(Role::Picking), None);
    assert_eq!(res[1].binding(Role::Kick), None);
    assert_eq!(res[1].binding(Role::Pad), Some(Pad));
    assert_eq!(res[1].binding(Role::Bass), Some(HeavyBass));
    // And visual-demo cannot use explain's pad.
    assert_eq!(res[0].binding(Role::Pad), None);
    assert_eq!(res[0].binding(Role::Picking), Some(Guitar));
}

#[test]
fn fallback_order() {
    let (arr, own) = arranged(REFERENCE, Feel::Spring, 7);
    let vd = 2; // visual-demo
    let res = resolve_sections(&arr, &[Guitar, Pad, SoftBass, Shaker], &own);
    assert_eq!(res[vd].binding(Role::Lead), Some(Guitar));
    assert_eq!(res[vd].binding(Role::ChordHits), Some(Guitar));
    assert_eq!(res[vd].binding(Role::Bass), Some(SoftBass));
    assert_eq!(res[vd].binding(Role::Kick), None);
    assert_eq!(res[vd].binding(Role::Rim), None);
    let res = resolve_sections(&arr, &[Mallet, Guitar, Flute], &own);
    assert_eq!(res[vd].binding(Role::Lead), Some(Flute));
    let res = resolve_sections(&arr, &[Guitar, Mallet], &own);
    assert_eq!(res[vd].binding(Role::Lead), Some(Mallet));
    assert_eq!(res[vd].binding(Role::Motif), Some(Mallet));
    let res = resolve_sections(&arr, &[Flute, Guitar], &own);
    assert_eq!(res[vd].binding(Role::Motif), Some(Flute));
    let res = resolve_sections(&arr, &[Mallet], &own);
    assert_eq!(res[vd].binding(Role::Picking), Some(Mallet));
    let roles = &res[vd].roles;
    let lead = roles.iter().find(|r| r.role == Role::Lead).unwrap();
    assert_eq!(lead.status, "substituted");
    let pad = roles.iter().find(|r| r.role == Role::Pad).unwrap();
    assert_eq!(pad.status, "dropped");
}

#[test]
fn bass_preference_follows_feel_and_section() {
    let plan = r#"[["explain","0:00-0:10"],["sponsor","0:10-0:20"]]"#;
    let (arr, own) = arranged(plan, Feel::Spring, 7);
    let res = resolve_sections(&arr, &Instrument::ALL, &own);
    assert_eq!(res[0].binding(Role::Bass), Some(HeavyBass));
    assert_eq!(res[1].binding(Role::Bass), Some(SoftBass));
    let (arr, own) = arranged(plan, Feel::Summer, 7);
    let res = resolve_sections(&arr, &Instrument::ALL, &own);
    assert_eq!(res[0].binding(Role::Bass), Some(SoftBass));
    // The other bass is the fallback.
    let res = resolve_sections(&arr, &[HeavyBass], &own);
    assert_eq!(res[0].binding(Role::Bass), Some(HeavyBass));
}

#[test]
fn empty_list_is_silent_with_a_warning() {
    let plan =
        r#"[["intro","0:00-0:08"],["reflect","0:08-0:16",[]],["outro-end-screen","0:16-0:24"]]"#;
    let (arr, own) = arranged(plan, Feel::Spring, 7);
    let res = resolve_sections(&arr, &Instrument::ALL, &own);
    assert!(res[1].roles.iter().all(|r| r.instrument.is_none()));
    assert!(res[1].warnings.iter().any(|w| w.contains("silent")));
    let vs = voices(&arr.events, &res);
    assert!(vs.iter().all(|v| v.event.section != 1));
    assert!(vs.iter().any(|v| v.event.section == 0));
}

#[test]
fn energy_warning_when_roles_are_dropped() {
    let plan = r#"[["reveal","0:00-0:10",["pad"]]]"#;
    let (arr, own) = arranged(plan, Feel::Spring, 7);
    let res = resolve_sections(&arr, &Instrument::ALL, &own);
    assert!(
        res[0].warnings.iter().any(|w| w.contains("energy")),
        "{:?}",
        res[0].warnings
    );
    let vs = voices(&arr.events, &res);
    assert!(!vs.is_empty() && vs.iter().all(|v| v.instrument == Pad));
}

#[test]
fn instruments_change_only_the_sound() {
    let (arr, own) = arranged(REFERENCE, Feel::Spring, 7);
    let full = resolve_sections(&arr, &Instrument::ALL, &own);
    let restricted = resolve_sections(&arr, &[Guitar, Pad, SoftBass, Shaker], &own);
    let a = voices(&arr.events, &full);
    let b = voices(&arr.events, &restricted);
    let lead_a: Vec<(u64, i32)> = a
        .iter()
        .filter(|v| v.bus == Bus::Lead)
        .map(|v| (v.event.id, v.event.midi))
        .collect();
    let lead_b: Vec<(u64, i32)> = b
        .iter()
        .filter(|v| v.bus == Bus::Lead)
        .map(|v| (v.event.id, v.event.midi))
        .collect();
    assert!(!lead_a.is_empty());
    assert_eq!(lead_a, lead_b);
    // Guitar plays the lead one octave up.
    for (va, vb) in a
        .iter()
        .filter(|v| v.bus == Bus::Lead)
        .zip(b.iter().filter(|v| v.bus == Bus::Lead))
    {
        assert_eq!(va.instrument, Flute);
        assert_eq!(vb.instrument, Guitar);
        assert_eq!(vb.midi, va.midi + 12);
    }
    // The arrangement itself never sees the instruments: re-arranging with a
    // per-section list gives the same notes.
    let plan = REFERENCE.replace(
        r#"["explain","0:24-0:40"]"#,
        r#"["explain","0:24-0:40",["pad","heavy-bass"]]"#,
    );
    let (arr2, _) = arranged(&plan, Feel::Spring, 7);
    assert_eq!(arr.events, arr2.events);
    assert_eq!(lead_notes(&arr), lead_notes(&arr2));
}

#[test]
fn speech_sections_carry_no_lead() {
    let plan =
        r#"[["cold-open-hook","0:00-0:06"],["explain","0:06-0:20"],["sponsor","0:20-0:30"]]"#;
    let (arr, _) = arranged(plan, Feel::Spring, 7);
    assert!(arr.events.iter().all(|e| e.role != Role::Lead));
}

#[test]
fn no_melody_drops_only_the_lead_role() {
    let (arr, _) = arranged(REFERENCE, Feel::Spring, 7);
    let nm = arr.events_without_lead();
    assert!(nm.iter().all(|e| e.role != Role::Lead));
    let others = arr.events.iter().filter(|e| e.role != Role::Lead).count();
    assert_eq!(nm.len(), others);
}

#[test]
fn transitions_follow_energy_delta() {
    let plan = r#"[["intro","0:00-0:08"],["visual-demo","0:08-0:16"],["explain","0:16-0:26"],
                   ["reveal","0:26-0:34"],["reflect","0:34-0:42"],["build","0:42-0:48"],
                   ["sponsor","0:48-0:56"],["title-sting","0:56-1:00"]]"#;
    let (arr, _) = arranged(plan, Feel::Spring, 7);
    let kinds: Vec<(&str, bool)> = arr.transitions.iter().map(|t| (t.kind, t.hit)).collect();
    assert_eq!(
        kinds,
        vec![
            ("pickup", false), // intro 3 -> visual-demo 4
            ("drop", false),   // visual-demo 4 -> explain 2
            ("run-up", true),  // explain 2 -> reveal 5, into reveal
            ("drop", false),   // reveal 5 -> reflect 1
            ("pickup", false), // reflect 1 -> build 2
            ("drop", true),    // build ends at 5 -> sponsor 2, out of build
            ("run-up", true),  // sponsor 2 -> title-sting 4.5
        ]
    );
    // The run-up is the 6-note figure, owned by the incoming section.
    let t = &arr.transitions[2];
    let run: Vec<i32> = arr
        .events
        .iter()
        .filter(|e| e.section == 3 && e.role == Role::Motif && e.time < t.time + 0.02)
        .map(|e| e.midi)
        .collect();
    assert_eq!(run, vec![74, 76, 78, 81, 83, 86]);
    // Drums rest for the last beat before a drop.
    let drop = &arr.transitions[1];
    assert!(!arr.events.iter().any(|e| e.section == 1
        && matches!(e.role, Role::Kick | Role::Shaker | Role::Rim)
        && e.time >= drop.time - arr.spb + 0.01
        && e.time < drop.time));
}

#[test]
fn title_sting_signature_is_the_same_every_time() {
    let plan =
        r#"[["title-sting","0:00-0:05"],["explain","0:05-0:15"],["title-sting","0:15-0:20"]]"#;
    let (arr, _) = arranged(plan, Feel::Spring, 11);
    let sig = |sec: usize| -> Vec<i32> {
        arr.events
            .iter()
            .filter(|e| e.section == sec && e.role == Role::Lead)
            .map(|e| e.midi)
            .collect()
    };
    assert!(!sig(0).is_empty());
    assert_eq!(sig(0), sig(2));
}

#[test]
fn melody_range_in_rendered_arrangement() {
    for seed in 0..10 {
        let (arr, _) = arranged(REFERENCE, Feel::Spring, seed);
        for e in arr.events.iter().filter(|e| e.role == Role::Lead) {
            assert!((74..=90).contains(&e.midi), "{e:?}");
        }
    }
}
