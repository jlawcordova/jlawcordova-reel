//! `cozy-score detect`: motion classification on synthetic data shaped like
//! the reference clip, and (when ffmpeg is on PATH) the whole command on a
//! synthetic video.

use cozy_score::detect::{classify, draft_plan, have_ffmpeg, measure, suggest_bpm};
use cozy_score::types::Feel;
use serde_json::Value;

const TICKS: [f64; 11] = [1.4, 1.8, 2.2, 2.4, 2.8, 3.0, 3.4, 3.8, 4.4, 4.8, 5.4];

/// 7 s at 30 fps: the room assembling (0.15-1.1 s, a staggered wave that
/// dips between pieces), 11 one-frame steps with 3.8 s the largest, the
/// room breaking apart (5.9-6.8 s) and a little noise.
fn reference_motion() -> Vec<f64> {
    let fps = 30.0;
    let n = 210;
    let mut m: Vec<f64> = (0..n).map(|i| 0.2 + 0.1 * ((i * 7) % 5) as f64).collect();
    for (i, v) in m.iter_mut().enumerate() {
        let t = i as f64 / fps;
        if (0.15..=1.1).contains(&t) {
            *v = if i % 6 == 1 {
                3.0
            } else {
                18.0 + (i % 4) as f64 * 3.0
            };
        }
        if (5.9..=6.8).contains(&t) {
            *v = 20.0 + (i % 3) as f64 * 4.0;
        }
    }
    for t in TICKS {
        let f = (t * fps).round() as usize;
        m[f] = if t == 3.8 { 16.0 } else { 9.0 + (f % 3) as f64 };
    }
    m
}

#[test]
fn windows_and_ticks_are_classified() {
    let d = classify(&reference_motion(), 30.0, 0.25);
    assert_eq!(d.windows.len(), 2, "{:?}", d.windows);
    let (a, b) = (d.windows[0], d.windows[1]);
    assert!(
        (a.start - 0.15).abs() <= 1.0 / 30.0 && (a.end - 1.1).abs() <= 1.0 / 30.0,
        "{a:?}"
    );
    assert!(
        (b.start - 5.9).abs() <= 1.0 / 30.0 && (b.end - 6.8).abs() <= 1.0 / 30.0,
        "{b:?}"
    );
    let ticks: Vec<f64> = d.ticks.iter().map(|t| t.time).collect();
    assert_eq!(ticks.len(), 11, "{ticks:?}");
    for (got, want) in ticks.iter().zip(TICKS) {
        assert!((got - want).abs() <= 1.0 / 30.0 + 1e-9, "{got} vs {want}");
    }
    let biggest = d
        .ticks
        .iter()
        .max_by(|x, y| x.motion.total_cmp(&y.motion))
        .unwrap();
    assert!((biggest.time - 3.8).abs() < 1e-9);
}

#[test]
fn the_draft_plan_follows_the_reference_clip() {
    let d = classify(&reference_motion(), 30.0, 0.25);
    let plan = draft_plan(&d, 7.0, Feel::Chip);
    assert_eq!(plan["bpm"], 150);
    let ev = plan["events"].as_array().unwrap();
    assert_eq!(ev[0]["kind"], "run");
    assert_eq!(ev[0]["direction"], "up");
    assert_eq!(ev[0]["riser"], true);
    assert!(
        (ev[0]["land"].as_f64().unwrap() - 1.2).abs() < 1e-9,
        "{}",
        ev[0]
    );
    assert_eq!(ev[1]["kind"], "ticks");
    assert_eq!(ev[1]["times"].as_array().unwrap().len(), 11);
    assert_eq!(ev[1]["accent"][0], 3.8);
    assert_eq!(ev[2]["direction"], "down");
    assert_eq!(ev[2]["riser"], "reverse");
    assert_eq!(ev[3]["kind"], "hit");
    assert_eq!(ev[3]["strength"], "pop");
    assert!((ev[3]["at"].as_f64().unwrap() - 6.8).abs() <= 1.0 / 30.0 + 1e-9);
    let kinds: Vec<&str> = plan["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s[0].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["build", "visual-demo", "outro-end-screen"]);
    assert_eq!(plan["sections"][0][1], "0:00.000-0:01.200");
    assert!(plan["notes"].as_array().unwrap().len() >= 4);
    // The draft is a valid plan.
    let text = serde_json::to_string(&plan).unwrap();
    cozy_score::plan::parse_plan(&text).unwrap();
}

#[test]
fn the_tempo_suggestion_prefers_the_feel() {
    // 75 and 150 BPM both fit these exactly; the chip feel takes 150.
    assert_eq!(suggest_bpm(&TICKS, Feel::Chip, 0.018), 150.0);
    // Spring has no tempo that fits, so the best fit anywhere wins.
    let b = suggest_bpm(&TICKS, Feel::Spring, 0.018);
    assert!(b == 75.0 || b == 150.0, "{b}");
}

#[test]
fn detect_runs_on_a_real_video() {
    if !have_ffmpeg() {
        eprintln!("skipped: ffmpeg is not on PATH");
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let clip = dir.join("detect-synthetic.mp4");
    let steps: Vec<String> = TICKS.iter().map(|t| format!("20*gte(t,{t})")).collect();
    let graph = format!(
        "[0][1]overlay=x='20+260*(t-0.15)':y=10:eval=frame:enable='between(t,0.15,1.1)'[a];\
         [a][2]overlay=x='10+{}':y=90:eval=frame[b];\
         [b][3]overlay=x='270-40*gte(t,3.8)':y=136:eval=frame[c];\
         [c][1]overlay=x='260-260*(t-5.9)':y=10:eval=frame:enable='between(t,5.9,6.8)',format=yuv420p",
        steps.join("+")
    );
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "color=c=black:s=320x180:r=30:d=7"])
        .args(["-f", "lavfi", "-i", "color=c=white:s=60x60:r=30:d=7"])
        .args(["-f", "lavfi", "-i", "color=c=0xc0ffc0:s=30x40:r=30:d=7"])
        .args(["-f", "lavfi", "-i", "color=c=white:s=30x40:r=30:d=7"])
        .args([
            "-filter_complex",
            &graph,
            "-c:v",
            "libx264",
            "-qp",
            "0",
            "-t",
            "7",
        ])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(status.success());
    let (motion, fps, duration) = measure(&clip).unwrap();
    assert_eq!(fps, 30.0);
    assert!((duration - 7.0).abs() < 0.05);
    let d = classify(&motion, fps, 0.25);
    let plan: Value = draft_plan(&d, duration, Feel::Chip);
    let ticks = plan["events"][1]["times"].as_array().unwrap();
    assert_eq!(ticks.len(), 11, "{plan}");
    for (got, want) in ticks.iter().zip(TICKS) {
        assert!(
            (got.as_f64().unwrap() - want).abs() <= 1.0 / 30.0 + 1e-9,
            "{got} vs {want}"
        );
    }
    assert_eq!(plan["events"][1]["accent"][0], 3.8);
    assert_eq!(plan["bpm"], 150);
}
