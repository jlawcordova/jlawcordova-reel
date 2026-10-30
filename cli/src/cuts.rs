//! `cut …` (spec §7.8): timing edits in cuts.yaml, and the lock.

use crate::files::{BeatCut, Cuts, LineCut, Tighten};
use crate::project::{Errors, Project};
use crate::storyboard::{Storyboard, Voice};
use crate::time::clock;
use crate::timeline::{self, LineAudio, LockInputs, Timeline};
use std::collections::BTreeMap;

/// The estimated timeline for the project: picked VO audio where it exists,
/// draft TTS in `tts` voice when given, and the measured speaking rate once
/// there are takes.
pub fn estimate(
    p: &Project,
    sb: &Storyboard,
    tts: Option<&str>,
) -> Result<(Timeline, Vec<String>), Errors> {
    let cuts = p.cuts()?;
    let picks = p.picks()?;
    let assets = p.assets()?;
    let wpm = crate::takes::reads_by_line(p, sb)
        .ok()
        .and_then(|(r, _, _)| timeline::measured_wpm(&r))
        .unwrap_or(p.config.speech.wpm);
    let audio = |l: &crate::storyboard::Line| -> Option<LineAudio> {
        if let Some(v) = tts {
            return crate::vo::draft_audio(p, l, v);
        }
        crate::vo::picked_audio(p, l, &picks)
    };
    Ok(timeline::estimate(
        sb, &p.config, &cuts, &assets, wpm, &audio,
    ))
}

/// Builds the locked timeline from the current inputs, without writing it.
pub fn build_locked(p: &Project, sb: &Storyboard) -> Result<(Timeline, Vec<String>), Errors> {
    let cuts = p.cuts()?;
    let picks = p.picks()?;
    let assets = p.assets()?;
    let (reads, _, _) = crate::takes::reads_by_line(p, sb)?;
    let takes = crate::takes::take_files(p)?;
    let vo = |l: &crate::storyboard::Line| crate::vo::picked_audio(p, l, &picks);
    timeline::lock(
        sb,
        &p.config,
        &cuts,
        &assets,
        &LockInputs {
            reads: &reads,
            takes: &takes,
            vo: &vo,
        },
    )
}

/// The locked timeline on disk, if any.
pub fn locked(p: &Project) -> Option<Timeline> {
    timeline::load(&p.timeline_path()).filter(|t| t.locked)
}

/// Whether the locked timeline no longer matches its inputs (None when not
/// locked).
pub fn locked_is_stale(p: &Project, sb: &Storyboard) -> Option<bool> {
    let t = locked(p)?;
    Some(match build_locked(p, sb) {
        Ok((fresh, _)) => fresh.inputs != t.inputs,
        Err(_) => true,
    })
}

/// The timeline to work from: the locked one if there is one, otherwise
/// the estimate.
pub fn current(p: &Project, sb: &Storyboard) -> Result<(Timeline, Vec<String>), Errors> {
    match locked(p) {
        Some(t) => {
            let mut w = Vec::new();
            if locked_is_stale(p, sb) == Some(true) {
                w.push("the locked timeline is out of date with the storyboard, cuts, takes or VO; run pixelreel cut unlock, then cut lock".into());
            }
            Ok((t, w))
        }
        None => estimate(p, sb, None),
    }
}

fn refuse_if_locked(p: &Project) -> Result<(), Errors> {
    if locked(p).is_some() {
        return Err(vec![
            "the timeline is locked; run pixelreel cut unlock first (then cut lock again)".into(),
        ]);
    }
    Ok(())
}

fn find_line<'a>(sb: &'a Storyboard, id: &str) -> Result<&'a crate::storyboard::Line, Errors> {
    sb.beats
        .iter()
        .flat_map(|b| b.lines.iter())
        .find(|l| l.id == id)
        .ok_or_else(|| {
            vec![format!(
                "no line \"{id}\" in storyboard.md (lines are <beat>.<n>, like hook.2)"
            )]
        })
}

fn check_beat(sb: &Storyboard, id: &str) -> Result<(), Errors> {
    if sb.beats.iter().any(|b| b.id == id) {
        Ok(())
    } else {
        Err(vec![format!("no beat \"{id}\" in storyboard.md")])
    }
}

fn save_line(
    p: &Project,
    cuts: &mut Cuts,
    id: &str,
    f: impl FnOnce(&mut LineCut),
) -> Result<(), Errors> {
    let e = cuts.lines.entry(id.to_string()).or_default();
    f(e);
    if e.is_empty() {
        cuts.lines.remove(id);
    }
    p.save_cuts(cuts)
}

fn save_beat(
    p: &Project,
    cuts: &mut Cuts,
    id: &str,
    f: impl FnOnce(&mut BeatCut),
) -> Result<(), Errors> {
    let e = cuts.beats.entry(id.to_string()).or_default();
    f(e);
    if e.is_empty() {
        cuts.beats.remove(id);
    }
    p.save_cuts(cuts)
}

pub fn tighten(
    p: &Project,
    max_gap: Option<f64>,
    keep_breath: Option<f64>,
) -> Result<String, Errors> {
    refuse_if_locked(p)?;
    let mut cuts = p.cuts()?;
    let mut t = cuts.tighten.unwrap_or_default();
    let max_gap = max_gap.unwrap_or(t.max_gap.unwrap_or(p.config.cut.max_gap));
    let keep_breath = keep_breath.unwrap_or(t.keep_breath.unwrap_or(p.config.cut.keep_breath));
    for (k, v) in [("--max-gap", max_gap), ("--keep-breath", keep_breath)] {
        if !(0.0..=2.0).contains(&v) {
            return Err(vec![format!("{k} must be 0 to 2 seconds")]);
        }
    }
    t = Tighten {
        max_gap: Some(max_gap),
        keep_breath: Some(keep_breath),
    };
    cuts.tighten = Some(t);
    p.save_cuts(&cuts)?;
    Ok(format!(
        "pauses up to {max_gap:.2} s are kept between back-to-back lines; {keep_breath:.2} s kept after each line"
    ))
}

/// `cut set <beat>.<line> --in/--out` or `cut set <beat> --length`.
pub fn set(
    p: &Project,
    sb: &Storyboard,
    target: &str,
    in_: Option<f64>,
    out: Option<f64>,
    length: Option<f64>,
) -> Result<String, Errors> {
    refuse_if_locked(p)?;
    let mut cuts = p.cuts()?;
    if target.contains('.') {
        find_line(sb, target)?;
        if length.is_some() {
            return Err(vec![
                "--length is for a beat; use --in and --out for a line".into(),
            ]);
        }
        if in_.is_none() && out.is_none() {
            return Err(vec!["give --in and/or --out (seconds, like -0.2)".into()]);
        }
        save_line(p, &mut cuts, target, |c| {
            if let Some(v) = in_ {
                c.in_ = (v != 0.0).then_some(v);
            }
            if let Some(v) = out {
                c.out = (v != 0.0).then_some(v);
            }
        })?;
        Ok(format!(
            "{target}: in {:+.2} s, out {:+.2} s",
            in_.unwrap_or(0.0),
            out.unwrap_or(0.0)
        ))
    } else {
        check_beat(sb, target)?;
        if in_.is_some() || out.is_some() {
            return Err(vec![
                "--in and --out are for a line (<beat>.<n>); use --length for a beat".into(),
            ]);
        }
        let Some(len) = length else {
            return Err(vec!["give --length (seconds, like +0.21 or -0.5)".into()]);
        };
        save_beat(p, &mut cuts, target, |c| {
            c.length = (len != 0.0).then_some(len)
        })?;
        Ok(format!("{target}: length {len:+.2} s"))
    }
}

pub fn take(p: &Project, sb: &Storyboard, line: &str, take: &str) -> Result<String, Errors> {
    refuse_if_locked(p)?;
    let l = find_line(sb, line)?;
    if l.voice != Voice::You {
        return Err(vec![format!(
            "{line} is a VO line; choose its take with pixelreel vo pick"
        )]);
    }
    let (reads, _, _) = crate::takes::reads_by_line(p, sb)?;
    if timeline::choose_read(line, &reads, Some(take)).is_none() {
        return Err(vec![format!("take {take} has no complete read of {line}")]);
    }
    let mut cuts = p.cuts()?;
    save_line(p, &mut cuts, line, |c| c.take = Some(take.to_string()))?;
    Ok(format!("{line}: using take {take}"))
}

pub fn lead(p: &Project, sb: &Storyboard, beat: &str, secs: f64) -> Result<String, Errors> {
    refuse_if_locked(p)?;
    check_beat(sb, beat)?;
    if sb.beats.first().is_some_and(|b| b.id == beat) && secs != 0.0 {
        return Err(vec![format!(
            "{beat} is the first beat, so it can't have a lead"
        )]);
    }
    if !(0.0..=10.0).contains(&secs) {
        return Err(vec!["a lead is 0 to 10 seconds".into()]);
    }
    let mut cuts = p.cuts()?;
    save_beat(p, &mut cuts, beat, |c| {
        c.lead = (secs != 0.0).then_some(secs)
    })?;
    Ok(format!(
        "{beat}: its picture starts {secs:.2} s after its voice"
    ))
}

pub fn lock(p: &Project, sb: &Storyboard) -> Result<(Vec<String>, Vec<String>), Errors> {
    let (t, warnings) = build_locked(p, sb)?;
    timeline::save(&p.timeline_path(), &t)?;
    let lines = t.lines().count();
    Ok((
        vec![format!(
            "locked {} beats, {lines} lines, {} -> build/timeline.json",
            t.beats.len(),
            clock(t.duration)
        )],
        warnings,
    ))
}

pub fn unlock(p: &Project) -> Result<String, Errors> {
    let path = p.timeline_path();
    if !path.exists() {
        return Ok("the timeline wasn't locked".into());
    }
    std::fs::remove_file(&path)
        .map_err(|e| vec![format!("can't remove {}: {e}", path.display())])?;
    Ok("unlocked; music and renders fitted to the old timeline are now out of date".into())
}

/// `cut show` as text rows.
pub fn show(t: &Timeline, beat: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!(
        "{:<14} {:<8} {:>9} {:>9}  {:>9} {:>9}  {}",
        "line", "take", "in", "out", "start", "end", "gap"
    ));
    let mut prev_end: Option<f64> = None;
    for b in t.beats.iter().filter(|b| beat.is_none_or(|x| x == b.id)) {
        out.push(format!(
            "{} ({}, {}) picture {}–{}",
            b.id,
            b.kind,
            b.picture,
            fmt_s(b.start),
            fmt_s(b.end)
        ));
        for l in &b.lines {
            let (take, i, o) = match &l.source {
                Some(s) => (
                    s.take.clone().unwrap_or_else(|| l.voice.clone()),
                    fmt_s(s.in_),
                    fmt_s(s.out),
                ),
                None => ("est.".into(), "-".into(), "-".into()),
            };
            let gap = prev_end.map_or(String::new(), |e| format!("{:+.2}", l.start - e));
            out.push(format!(
                "  {:<12} {:<8} {:>9} {:>9}  {:>9} {:>9}  {}",
                l.id,
                take,
                i,
                o,
                fmt_s(l.start),
                fmt_s(l.end),
                gap
            ));
            prev_end = Some(l.end);
        }
    }
    out
}

fn fmt_s(t: f64) -> String {
    format!("{t:.3}")
}

/// Line ids with their read counts, for `status`.
pub fn read_counts(p: &Project, sb: &Storyboard) -> BTreeMap<String, (usize, usize)> {
    match crate::takes::reads_by_line(p, sb) {
        Ok((r, _, _)) => r
            .into_iter()
            .map(|(k, v)| {
                let c = v.iter().filter(|x| x.complete).count();
                (k, (v.len(), c))
            })
            .collect(),
        Err(_) => BTreeMap::new(),
    }
}
