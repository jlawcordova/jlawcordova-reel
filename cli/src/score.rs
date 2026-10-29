//! Music: the cozy-score plan from the timeline, renders and the pick
//! (spec §7.13–7.15).

use crate::project::{Errors, Project};
use crate::timeline::Timeline;
use cozy_score::types::Instrument;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The plan: neighbouring beats of the same type merged into one section,
/// with `[startSeconds, endSeconds]` spans.
pub fn plan(t: &Timeline) -> Value {
    let mut sections: Vec<(String, f64, f64)> = Vec::new();
    for b in &t.beats {
        match sections.last_mut() {
            Some(last) if last.0 == b.kind => last.2 = b.end,
            _ => sections.push((b.kind.clone(), b.start, b.end)),
        }
    }
    Value::Array(
        sections
            .into_iter()
            .map(|(k, s, e)| json!([k, [s, e]]))
            .collect(),
    )
}

/// The hash a rendered seed records, so a changed plan shows as stale.
pub fn plan_hash(plan: &Value, feel: &str) -> String {
    crate::hash::of_json(&json!({ "plan": plan, "feel": feel }))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedMeta {
    pub seed: u64,
    pub feel: String,
    pub instruments: Option<String>,
    pub plan_hash: String,
    /// Rendered from a locked timeline (not estimates).
    pub locked: bool,
    pub bpm: f64,
}

pub fn seed_dir(p: &Project, seed: u64) -> PathBuf {
    p.score_dir().join(format!("seed-{seed}"))
}

pub fn seed_meta(p: &Project, seed: u64) -> Option<SeedMeta> {
    let text = std::fs::read_to_string(seed_dir(p, seed).join("pixelreel.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes `build/score/plan.json` after validating it with cozy-score.
pub fn write_plan(p: &Project, t: &Timeline) -> Result<Value, Errors> {
    let plan = plan(t);
    let text = serde_json::to_string_pretty(&plan).map_err(|e| vec![e.to_string()])?;
    cozy_score::plan::parse_plan(&text)?;
    crate::files::write_atomic(
        &p.score_dir().join("plan.json"),
        format!("{text}\n").as_bytes(),
    )?;
    Ok(plan)
}

/// Renders each seed in turn. Returns lines to print and warnings.
pub fn render(
    p: &Project,
    t: &Timeline,
    seeds: &[u64],
    instruments: Option<&str>,
    stems: bool,
) -> Result<(Vec<String>, Vec<String>), Errors> {
    let plan = write_plan(p, t)?;
    let specs = cozy_score::plan::parse_plan(&plan.to_string())?;
    let score: Vec<Instrument> = match instruments {
        Some(s) => cozy_score::plan::parse_instrument_arg(s)?,
        None => Instrument::ALL.to_vec(),
    };
    let feel = p.config.feel();
    let hash = plan_hash(&plan, &p.config.feel);
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for &seed in seeds {
        let started = std::time::Instant::now();
        let r = cozy_score::render::render(&cozy_score::render::Request {
            specs: &specs,
            feel,
            seed,
            instruments: score.clone(),
            stems,
            gain_from: None,
        })?;
        let dir = seed_dir(p, seed);
        let w = |name: &str, bytes: &[u8]| crate::files::write_atomic(&dir.join(name), bytes);
        w("full.wav", &cozy_score::render::wav_bytes(&r.full))?;
        w(
            "no-melody.wav",
            &cozy_score::render::wav_bytes(&r.no_melody),
        )?;
        if stems {
            for (id, st) in &r.stems {
                w(
                    &format!("stems/{id}.wav"),
                    &cozy_score::render::wav_bytes(st),
                )?;
            }
        }
        let cue = serde_json::to_string_pretty(&r.cue).map_err(|e| vec![e.to_string()])?;
        w("cue.json", format!("{cue}\n").as_bytes())?;
        let meta = SeedMeta {
            seed,
            feel: p.config.feel.clone(),
            instruments: instruments.map(str::to_string),
            plan_hash: hash.clone(),
            locked: t.locked,
            bpm: r.cue.bpm,
        };
        w(
            "pixelreel.json",
            format!(
                "{}\n",
                serde_json::to_string_pretty(&meta).unwrap_or_default()
            )
            .as_bytes(),
        )?;
        for n in &r.notes {
            warnings.push(format!("seed {seed}: {n}"));
        }
        let cue_v = serde_json::to_value(&r.cue).unwrap_or_default();
        warnings.extend(snap_warnings(t, &cue_v));
        out.push(format!(
            "seed {seed}: {:.1} s at {} BPM in {:.1} s -> {}",
            r.cue.duration_s,
            r.cue.bpm,
            started.elapsed().as_secs_f64(),
            dir.strip_prefix(&p.root).unwrap_or(&dir).display()
        ));
    }
    Ok((out, warnings))
}

/// Music hits that cozy-score's beat snapping moved more than 0.1 s from
/// their cut, at `title-sting` and `reveal`, with the change that would
/// line them up (spec §7.14).
pub fn snap_warnings(t: &Timeline, cue: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let seed = cue["seed"].as_u64().unwrap_or(0);
    for s in cue["sections"].as_array().into_iter().flatten() {
        let kind = s["type"].as_str().unwrap_or("");
        if kind != "title-sting" && kind != "reveal" {
            continue;
        }
        let off = s["start_offset_s"].as_f64().unwrap_or(0.0);
        let req = s["requested_start_s"].as_f64().unwrap_or(0.0);
        if off.abs() <= 0.1 || req <= 0.0 {
            continue;
        }
        let Some(i) = t.beats.iter().position(|b| (b.start - req).abs() < 1e-3) else {
            continue;
        };
        if i == 0 {
            continue;
        }
        let prev = &t.beats[i - 1].id;
        let fix = format!("pixelreel cut set {prev} --length {off:+.2}");
        let fix = if t.locked {
            format!("pixelreel cut unlock, {fix}, pixelreel cut lock")
        } else {
            fix
        };
        out.push(format!(
            "seed {seed}: the {kind} music starts {:.2} s {} the cut at {}; try: {fix}, then render the score again",
            off.abs(),
            if off > 0.0 { "after" } else { "before" },
            crate::time::clock(req)
        ));
    }
    out
}

/// The picked seed, if it has been rendered.
pub fn picked(p: &Project) -> Result<Option<u64>, Errors> {
    Ok(p.picks()?.score.map(|s| s.seed))
}

/// Records a pick (spec §7.15).
pub fn pick(p: &Project, seed: u64) -> Result<(), Errors> {
    if !seed_dir(p, seed).join("full.wav").exists() {
        return Err(vec![format!(
            "seed {seed} hasn't been rendered; run pixelreel score render --seeds {seed}"
        )]);
    }
    let mut picks = p.picks()?;
    picks.score = Some(crate::files::ScorePick { seed });
    p.save_picks(&picks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::TBeat;

    fn beat(id: &str, kind: &str, start: f64, end: f64) -> TBeat {
        TBeat {
            id: id.into(),
            kind: kind.into(),
            picture: "card".into(),
            start,
            end,
            chapter: None,
            voice_start: start,
            voice_end: end,
            lines: vec![],
        }
    }

    fn timeline(beats: Vec<TBeat>) -> Timeline {
        Timeline {
            pixelreel: String::new(),
            locked: true,
            inputs: String::new(),
            fps: 30,
            duration: beats.last().map_or(0.0, |b| b.end),
            beats,
            cues: Default::default(),
        }
    }

    #[test]
    fn merges_neighbouring_beats_of_a_type_and_validates() {
        let t = timeline(vec![
            beat("hook", "cold-open-hook", 0.0, 8.0),
            beat("tease", "cold-open-hook", 8.0, 14.0),
            beat("title", "title-sting", 14.0, 18.0),
            beat("a", "explain", 18.0, 30.0),
            beat("b", "visual-demo", 30.0, 40.0),
            beat("c", "explain", 40.0, 50.0),
        ]);
        let p = plan(&t);
        assert_eq!(
            p,
            json!([
                ["cold-open-hook", [0.0, 14.0]],
                ["title-sting", [14.0, 18.0]],
                ["explain", [18.0, 30.0]],
                ["visual-demo", [30.0, 40.0]],
                ["explain", [40.0, 50.0]]
            ])
        );
        assert!(cozy_score::plan::parse_plan(&p.to_string()).is_ok());
    }

    #[test]
    fn suggests_a_fix_for_a_late_music_hit() {
        let t = timeline(vec![
            beat("hook", "cold-open-hook", 0.0, 14.0),
            beat("title", "title-sting", 14.0, 18.0),
        ]);
        let cue = json!({"seed": 7, "sections": [
            {"type": "cold-open-hook", "requested_start_s": 0.0, "start_offset_s": 0.0},
            {"type": "title-sting", "requested_start_s": 14.0, "start_offset_s": 0.21}
        ]});
        let w = snap_warnings(&t, &cue);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("0.21 s after the cut at 0:14"), "{}", w[0]);
        assert!(w[0].contains("cut set hook --length +0.21"), "{}", w[0]);
        assert!(w[0].contains("cut unlock"), "{}", w[0]);
    }
}
