//! `status` (spec §7.17) and `check --youtube` (spec §7.18).

use crate::project::{Errors, Project};
use crate::storyboard::{Picture, Storyboard, Voice};
use crate::timeline::Timeline;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub struct Row {
    pub beat: String,
    pub script: String,
    pub voice: String,
    pub cut: String,
    pub picture: String,
    pub music: String,
}

#[derive(Serialize)]
pub struct Status {
    pub rows: Vec<Row>,
    pub locked: bool,
    pub locked_stale: bool,
    pub score_seed: Option<u64>,
    pub score_stale: bool,
}

const DONE: &str = "✓";
const NONE: &str = "–";
const STALE: &str = "stale";

/// Whether the picked score was rendered from the plan the current
/// timeline gives.
pub fn score_state(p: &Project, t: &Timeline) -> Result<(Option<u64>, bool), Errors> {
    let Some(seed) = crate::score::picked(p)? else {
        return Ok((None, false));
    };
    let plan = crate::score::plan(t);
    let fresh = crate::score::seed_meta(p, seed).is_some_and(|m| {
        m.plan_hash == crate::score::plan_hash(&plan, &p.config.feel) && m.locked == t.locked
    });
    Ok((Some(seed), !fresh))
}

pub fn status(p: &Project, sb: &Storyboard) -> Result<Status, Errors> {
    let picks = p.picks()?;
    let assets = p.assets()?;
    let reads = crate::cuts::read_counts(p, sb);
    let locked_t = crate::cuts::locked(p);
    let locked_stale = crate::cuts::locked_is_stale(p, sb).unwrap_or(false);
    let (t, _) = crate::cuts::current(p, sb)?;
    let (seed, score_stale) = score_state(p, &t)?;
    let music = match (seed, score_stale) {
        (None, _) => NONE.to_string(),
        (Some(_), true) => STALE.to_string(),
        (Some(s), false) => format!("{DONE} {s}"),
    };
    let doc_ok = p.pixellab_document().is_some_and(|d| d.is_dir());
    let mut cache = p.hash_cache();
    let mut rows = Vec::new();
    for b in &sb.beats {
        let you: Vec<_> = b.lines.iter().filter(|l| l.voice == Voice::You).collect();
        let vo: Vec<_> = b
            .lines
            .iter()
            .filter(|l| matches!(l.voice, Voice::Vo(_)))
            .collect();
        let you_done = you
            .iter()
            .filter(|l| reads.get(&l.id).is_some_and(|r| r.1 > 0))
            .count();
        let vo_done = vo
            .iter()
            .filter(|l| crate::vo::picked_audio(p, l, &picks).is_some())
            .count();
        let total = you.len() + vo.len();
        let done = you_done + vo_done;
        let voice = if total == 0 {
            NONE.to_string()
        } else if done == total {
            DONE.to_string()
        } else {
            format!("{done}/{total}")
        };
        let cut = match (&locked_t, locked_stale) {
            (None, _) => NONE,
            (Some(_), true) => STALE,
            (Some(_), false) => DONE,
        }
        .to_string();
        let picture = match &b.picture {
            Picture::Card => "card".to_string(),
            Picture::Face => {
                if !you.is_empty() && you_done == you.len() {
                    DONE.into()
                } else {
                    NONE.into()
                }
            }
            Picture::Scene(_) => if doc_ok { DONE } else { NONE }.into(),
            Picture::Asset(id) => match assets.get(id) {
                None => NONE.into(),
                Some(a) => match cache.file(&p.resolve(&a.path)) {
                    Ok(h) if h == a.sha256 => DONE.into(),
                    Ok(_) => STALE.into(),
                    Err(_) => "missing".into(),
                },
            },
        };
        rows.push(Row {
            beat: b.id.clone(),
            script: DONE.into(),
            voice,
            cut,
            picture,
            music: music.clone(),
        });
    }
    cache.save();
    Ok(Status {
        rows,
        locked: locked_t.is_some(),
        locked_stale,
        score_seed: seed,
        score_stale,
    })
}

pub fn status_text(s: &Status) -> Vec<String> {
    let w = s
        .rows
        .iter()
        .map(|r| r.beat.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = vec![format!(
        "{:<w$}  {:<7} {:<7} {:<7} {:<8} {}",
        "beat", "script", "voice", "cut", "picture", "music"
    )];
    for r in &s.rows {
        out.push(format!(
            "{:<w$}  {:<7} {:<7} {:<7} {:<8} {}",
            r.beat, r.script, r.voice, r.cut, r.picture, r.music
        ));
    }
    out.push(String::new());
    out.push(match (s.locked, s.locked_stale) {
        (false, _) => "timeline: not locked (times are estimates)".into(),
        (true, false) => "timeline: locked".into(),
        (true, true) => {
            "timeline: locked, but out of date; run pixelreel cut unlock, then cut lock".into()
        }
    });
    out.push(match (s.score_seed, s.score_stale) {
        (None, _) => "music: no seed picked".into(),
        (Some(seed), false) => format!("music: seed {seed}"),
        (Some(seed), true) => format!("music: seed {seed}, but rendered from another plan; run pixelreel score render --seeds {seed}"),
    });
    out
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Check {
    pub name: String,
    /// "pass", "fail" or "warn".
    pub level: String,
    pub detail: String,
}

fn check(name: &str, ok: bool, fail_level: &str, detail: String) -> Check {
    Check {
        name: name.into(),
        level: if ok { "pass".into() } else { fail_level.into() },
        detail,
    }
}

/// The YouTube checks on a timeline and a final render's report.
pub fn youtube_checks(
    t: &Timeline,
    report: &Value,
    target_lufs: f64,
    ceiling_db: f64,
    fresh: (bool, String),
    snaps: &[String],
) -> Vec<Check> {
    let mut out = Vec::new();
    // Chapters.
    let chapters = crate::captions::chapters(t);
    let mut lens = Vec::new();
    for (i, (s, _)) in chapters.iter().enumerate() {
        let end = chapters.get(i + 1).map_or(t.duration, |c| c.0);
        lens.push(end - s);
    }
    let shortest = lens.iter().copied().fold(f64::INFINITY, f64::min);
    let first0 = chapters.first().is_some_and(|c| c.0 < 1e-6);
    let ok = first0 && chapters.len() >= 3 && shortest >= 10.0;
    out.push(check(
        "chapters",
        ok,
        "fail",
        if chapters.is_empty() {
            "no chapters; add chapter: to beats".into()
        } else {
            format!(
                "{}, first at {}, shortest {}{}",
                chapters.len(),
                crate::time::clock(chapters[0].0),
                crate::time::clock(shortest),
                if ok {
                    String::new()
                } else {
                    " (need the first at 0:00, at least 3, each 10 s or more)".into()
                }
            )
        },
    ));
    // End screen.
    let last = t.beats.last();
    let end_len = last.map_or(0.0, |b| b.end - b.start);
    let is_outro = last.is_some_and(|b| b.kind == "outro-end-screen");
    let ok = is_outro && (5.0 - 1e-6..=20.0 + 1e-6).contains(&end_len) && t.duration >= 25.0;
    out.push(check(
        "end screen",
        ok,
        "fail",
        if is_outro {
            format!(
                "{} {:.1} s at {}–{}{}",
                last.unwrap().id,
                end_len,
                crate::time::clock(last.unwrap().start),
                crate::time::clock(last.unwrap().end),
                if ok {
                    ""
                } else {
                    " (needs 5–20 s, in a video of at least 25 s)"
                }
            )
        } else {
            "the last beat isn't outro-end-screen".into()
        },
    ));
    // Loudness.
    let lufs = report["master"]["integrated_lufs"].as_f64();
    let tp = report["master"]["true_peak_dbtp"].as_f64().unwrap_or(0.0);
    let ok = lufs.is_some_and(|l| (l - target_lufs).abs() <= 1.0) && tp <= ceiling_db + 0.05;
    out.push(check(
        "loudness",
        ok,
        "fail",
        format!(
            "{} LUFS, {tp:.1} dBTP (target {target_lufs} ± 1 LUFS, at most {ceiling_db} dBTP)",
            lufs.map_or("n/a".into(), |l| format!("{l:.1}"))
        ),
    ));
    // Captions.
    let lines = t.lines().count();
    let caps = crate::captions::captions(t);
    let covered = t
        .lines()
        .filter(|l| caps.iter().any(|c| c.start < l.end && c.end > l.start))
        .count();
    out.push(check(
        "captions",
        covered == lines,
        "fail",
        format!("{covered}/{lines} lines"),
    ));
    // Media.
    out.push(check("media", fresh.0, "fail", fresh.1));
    // Placeholders.
    let cards: Vec<&str> = t
        .beats
        .iter()
        .filter(|b| b.picture == "card")
        .map(|b| b.id.as_str())
        .collect();
    out.push(check(
        "placeholders",
        cards.is_empty(),
        "fail",
        if cards.is_empty() {
            "no cards".into()
        } else {
            format!("still cards: {}", cards.join(", "))
        },
    ));
    // Music hits.
    out.push(check(
        "music hits",
        snaps.is_empty(),
        "warn",
        if snaps.is_empty() {
            "title-sting and reveal hits land on their cuts".into()
        } else {
            snaps.join("; ")
        },
    ));
    // Hook.
    let hook: f64 = t
        .beats
        .iter()
        .take_while(|b| b.kind == "cold-open-hook")
        .map(|b| b.end - b.start)
        .sum();
    out.push(check(
        "hook",
        hook > 0.0 && hook <= 30.0,
        "warn",
        if hook > 0.0 {
            format!("cold open {:.1} s", hook)
        } else {
            "the video doesn't open with a cold-open-hook beat".into()
        },
    ));
    out
}

/// `check --youtube` on the latest final render.
pub fn check_youtube(p: &Project, sb: &Storyboard) -> Result<Vec<Check>, Errors> {
    let report_path = p.render_dir("final").join("render-report.json");
    let report: Value = std::fs::read_to_string(&report_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| {
            vec!["there's no final render to check; run pixelreel render --final".to_string()]
        })?;
    let t = crate::cuts::locked(p).ok_or_else(|| vec!["the timeline isn't locked".to_string()])?;
    let mut problems = Vec::new();
    if crate::cuts::locked_is_stale(p, sb) == Some(true) {
        problems.push("the locked timeline is out of date".to_string());
    }
    if report["timeline"].as_str() != Some(t.inputs.as_str()) {
        problems.push("the render is of another timeline".into());
    }
    let (seed, score_stale) = score_state(p, &t)?;
    if score_stale {
        problems.push("the picked score is out of date".into());
    }
    if seed.is_some() && report["score"]["seed"].as_u64() != seed {
        problems.push("the render used another score seed".into());
    }
    let assets = p.assets()?;
    let mut cache = p.hash_cache();
    for (id, a) in &assets {
        match cache.file(&p.resolve(&a.path)) {
            Ok(h) if h == a.sha256 => {}
            Ok(_) => problems.push(format!("asset {id} changed since it was added")),
            Err(_) => problems.push(format!("asset {id} is missing")),
        }
    }
    cache.save();
    let fresh = if problems.is_empty() {
        (true, "everything is up to date".to_string())
    } else {
        (false, problems.join("; "))
    };
    let snaps = match seed {
        Some(s) => std::fs::read_to_string(crate::score::seed_dir(p, s).join("cue.json"))
            .ok()
            .and_then(|x| serde_json::from_str::<Value>(&x).ok())
            .map(|cue| crate::score::snap_warnings(&t, &cue))
            .unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(youtube_checks(
        &t,
        &report,
        p.config.mix.target_lufs,
        p.config.mix.true_peak_dbtp,
        fresh,
        &snaps,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::TBeat;
    use serde_json::json;

    fn beat(id: &str, kind: &str, s: f64, e: f64, chapter: Option<&str>) -> TBeat {
        TBeat {
            id: id.into(),
            kind: kind.into(),
            picture: "face".into(),
            start: s,
            end: e,
            chapter: chapter.map(str::to_string),
            voice_start: s,
            voice_end: e,
            lines: vec![],
        }
    }

    fn tl(beats: Vec<TBeat>) -> Timeline {
        Timeline {
            pixelreel: String::new(),
            locked: true,
            inputs: String::new(),
            fps: 30,
            duration: beats.last().unwrap().end,
            beats,
            cues: Default::default(),
        }
    }

    fn level<'a>(c: &'a [Check], name: &str) -> &'a str {
        &c.iter().find(|x| x.name == name).unwrap().level
    }

    #[test]
    fn a_good_video_passes() {
        let t = tl(vec![
            beat("hook", "cold-open-hook", 0.0, 12.0, Some("Hook")),
            beat("a", "explain", 12.0, 40.0, Some("A")),
            beat("b", "explain", 40.0, 60.0, Some("B")),
            beat("end", "outro-end-screen", 60.0, 80.0, None),
        ]);
        let r = json!({"master": {"integrated_lufs": -14.1, "true_peak_dbtp": -1.3}});
        let c = youtube_checks(&t, &r, -14.0, -1.0, (true, "ok".into()), &[]);
        assert!(c.iter().all(|x| x.level == "pass"), "{c:?}");
    }

    #[test]
    fn each_rule_fails_on_its_own() {
        let t = tl(vec![
            beat("a", "explain", 0.0, 12.0, None),
            beat("b", "explain", 12.0, 20.0, Some("B")),
            beat("end", "outro-end-screen", 20.0, 45.0, None),
        ]);
        let r = json!({"master": {"integrated_lufs": -16.5, "true_peak_dbtp": -0.2}});
        let c = youtube_checks(
            &t,
            &r,
            -14.0,
            -1.0,
            (false, "stale".into()),
            &["late".into()],
        );
        for (name, lvl) in [
            ("chapters", "fail"),
            ("end screen", "fail"),
            ("loudness", "fail"),
            ("media", "fail"),
            ("music hits", "warn"),
            ("hook", "warn"),
        ] {
            assert_eq!(level(&c, name), lvl, "{name}: {c:?}");
        }
        assert_eq!(level(&c, "captions"), "pass");
    }
}
