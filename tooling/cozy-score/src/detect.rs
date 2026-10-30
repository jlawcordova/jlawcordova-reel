//! `cozy-score detect`: drafts a plan from a video's motion.
//!
//! Motion is the mean luma of the difference between consecutive frames
//! (ffmpeg's `tblend` + `signalstats`). Runs of at least 4 frames above a
//! threshold are windows (a scene assembling, transforming or breaking);
//! isolated 1-2 frame spikes outside them are ticks (sprite steps, pops).
//! This is the fallback for videos made outside pixellab: pixellab scenes
//! and sequences have exact times (`entrance` events, `from-pixellab`).

use crate::pixellab::num;
use crate::types::Feel;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

/// A run of frames with motion, by frame index (inclusive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub first: usize,
    pub last: usize,
    /// Seconds: the first frame's time, and the last frame's.
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub frame: usize,
    pub time: f64,
    pub motion: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub fps: f64,
    pub threshold: f64,
    pub p99: f64,
    pub windows: Vec<Window>,
    pub ticks: Vec<Tick>,
}

/// Frames a window needs.
pub const MIN_WINDOW: usize = 4;
/// Ticks are runs of at most this many frames.
pub const MAX_TICK: usize = 2;

/// The `q` quantile of `xs` (nearest rank).
fn quantile(xs: &[f64], q: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let i = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
    v[i]
}

/// Classifies per-frame motion (`motion[i]` at time `i / fps`). The
/// threshold is `ratio` x the 99th percentile. Within a window, gaps of one
/// quiet frame are bridged, since a staggered wave dips between pieces.
pub fn classify(motion: &[f64], fps: f64, ratio: f64) -> Detection {
    let p99 = quantile(motion, 0.99);
    let threshold = ratio * p99;
    let above: Vec<bool> = motion.iter().map(|&m| m > threshold && m > 0.0).collect();
    // Runs above the threshold.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < above.len() {
        if above[i] {
            let s = i;
            while i + 1 < above.len() && above[i + 1] {
                i += 1;
            }
            runs.push((s, i));
        }
        i += 1;
    }
    // Bridge one-frame gaps when the merge makes a window.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in runs {
        if let Some(last) = merged.last_mut() {
            let joined = r.0 <= last.1 + 2;
            if joined
                && (r.1 - last.0 + 1 >= MIN_WINDOW)
                && (last.1 - last.0 + 1 > MAX_TICK || r.1 - r.0 + 1 > MAX_TICK)
            {
                last.1 = r.1;
                continue;
            }
        }
        merged.push(r);
    }
    let t = |f: usize| f as f64 / fps;
    let mut windows = Vec::new();
    let mut ticks = Vec::new();
    for (a, b) in merged {
        let len = b - a + 1;
        if len >= MIN_WINDOW {
            windows.push(Window {
                first: a,
                last: b,
                start: t(a),
                end: t(b),
            });
        } else if len <= MAX_TICK {
            // The tick sits on its strongest frame.
            let f = (a..=b)
                .max_by(|&x, &y| motion[x].total_cmp(&motion[y]).then(y.cmp(&x)))
                .unwrap_or(a);
            ticks.push(Tick {
                frame: f,
                time: t(f),
                motion: motion[f],
            });
        }
    }
    Detection {
        fps,
        threshold,
        p99,
        windows,
        ticks,
    }
}

/// The BPM (0.1 steps from 60 to 200) whose 1/16 grid best fits `times`,
/// preferring the feel's range: a tempo there wins whenever it fits every
/// time to within `tol` seconds.
pub fn suggest_bpm(times: &[f64], feel: Feel, tol: f64) -> f64 {
    let (lo, hi) = feel.bpm_range();
    let center = feel.bpm_center();
    let err = |bpm: f64| -> (f64, f64) {
        let g = 15.0 / bpm;
        let mut worst = 0.0f64;
        let mut sq = 0.0;
        for &t in times {
            let d = (t - (t / g).round() * g).abs();
            worst = worst.max(d);
            sq += d * d;
        }
        (worst, sq)
    };
    let all: Vec<f64> = (600..=2000).map(|i| i as f64 / 10.0).collect();
    let best = |list: &[f64]| -> Option<f64> {
        list.iter().copied().min_by(|&a, &b| {
            let (ea, eb) = (err(a).1, err(b).1);
            ea.total_cmp(&eb)
                .then((a - center).abs().total_cmp(&(b - center).abs()))
        })
    };
    let fits: Vec<f64> = all.iter().copied().filter(|&b| err(b).0 <= tol).collect();
    let in_range: Vec<f64> = fits
        .iter()
        .copied()
        .filter(|b| (lo..=hi).contains(b))
        .collect();
    if times.is_empty() {
        return center;
    }
    best(&in_range)
        .or_else(|| best(&fits))
        .or_else(|| {
            let range: Vec<f64> = all
                .iter()
                .copied()
                .filter(|b| (lo..=hi).contains(b))
                .collect();
            best(&range)
        })
        .unwrap_or(center)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn snap_after(t: f64, g: f64) -> f64 {
    // The first 1/16 at least half a sixteenth after `t`.
    ((t + 0.5 * g) / g).ceil() * g
}

/// Drafts a plan from a detection and the clip's length.
pub fn draft_plan(d: &Detection, duration: f64, feel: Feel) -> Value {
    let tick_times: Vec<f64> = d.ticks.iter().map(|t| t.time).collect();
    let tol = 0.5 / d.fps + 0.002;
    let bpm = suggest_bpm(&tick_times, feel, tol);
    let g = 15.0 / bpm;
    let mut notes: Vec<String> = vec![
        format!(
            "Drafted by cozy-score detect from frame-difference motion at {} fps: threshold {:.3} (25% of the 99th percentile, {:.3}).",
            round3(d.fps),
            d.threshold,
            d.p99
        ),
        "Edit freely: these are guesses. pixellab scenes and sequences have exact times (entrance events, cozy-score from-pixellab).".into(),
    ];
    let mut events: Vec<Value> = Vec::new();
    let mut sections: Vec<Value> = Vec::new();
    let first = d.windows.first().copied();
    let last = d.windows.last().copied().filter(|w| Some(*w) != first);
    let mut cursor = 0.0;
    if let Some(w) = first {
        let land = snap_after(w.end, g);
        let count = (((land - w.start) / g).floor() as usize).clamp(3, 32);
        events.push(json!({
            "kind": "run", "direction": "up", "start": round3(w.start), "count": count,
            "step": "1/16", "land": round3(land), "riser": true
        }));
        notes.push(format!(
            "Motion window {:.3}-{:.3} s (frames {}-{}): the scene assembles, so an up run lands on a hit at {:.3} s, the first 1/16 after it.",
            w.start, w.end, w.first, w.last, land
        ));
        if land < duration {
            sections.push(json!(["build", span(0.0, land)]));
            cursor = land;
        }
    }
    let ticks: Vec<&Tick> = d.ticks.iter().collect();
    if !ticks.is_empty() {
        let biggest = ticks
            .iter()
            .max_by(|a, b| a.motion.total_cmp(&b.motion).then(b.frame.cmp(&a.frame)))
            .map(|t| t.time);
        events.push(json!({
            "kind": "ticks",
            "times": ticks.iter().map(|t| num(round3(t.time))).collect::<Vec<_>>(),
            "accent": biggest.map(|b| vec![num(round3(b))]).unwrap_or_default()
        }));
        notes.push(format!(
            "{} isolated motion spikes (1-2 frames, outside the windows): ticks at {}; the largest, at {:.3} s, is the accent.",
            ticks.len(),
            ticks.iter().map(|t| format!("{:.3}", t.time)).collect::<Vec<_>>().join(", "),
            biggest.unwrap_or(0.0)
        ));
    }
    if let Some(w) = last {
        let stop = round3(w.end);
        let count = (((w.end - w.start) / g).floor() as usize).clamp(3, 32);
        events.push(json!({
            "kind": "run", "direction": "down", "start": round3(w.start), "count": count,
            "step": "1/16", "riser": "reverse"
        }));
        events.push(json!({"kind": "hit", "at": stop, "strength": "pop"}));
        notes.push(format!(
            "Motion window {:.3}-{:.3} s (frames {}-{}): the scene breaks apart, so a down run with a reverse swell, and a pop where the motion stops.",
            w.start, w.end, w.first, w.last
        ));
        if w.start > cursor + 0.05 {
            sections.push(json!(["visual-demo", span(cursor, w.start)]));
            cursor = w.start;
        }
        sections.push(json!(["outro-end-screen", span(cursor, duration)]));
    } else {
        sections.push(json!(["visual-demo", span(cursor, duration)]));
    }
    notes.push(format!(
        "Tempo {bpm} BPM: its 1/16 grid ({:.4} s) fits the ticks best, preferring the {} feel's {}-{} BPM.",
        g,
        feel.id(),
        feel.bpm_range().0,
        feel.bpm_range().1
    ));
    json!({
        "feel": feel.id(),
        "bpm": num(bpm),
        "notes": notes,
        "sections": sections,
        "events": events,
    })
}

/// A section timespan as "m:ss.sss-m:ss.sss".
fn span(a: f64, b: f64) -> String {
    let fmt = |t: f64| {
        let ms = (t * 1000.0).round() as u64;
        format!("{}:{:02}.{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000)
    };
    format!("{}-{}", fmt(a), fmt(b))
}

/// Parses ffmpeg's `metadata=print` log: (frame index, YAVG) pairs.
pub fn parse_motion_log(log: &str) -> Vec<(usize, f64)> {
    let mut out = Vec::new();
    let mut frame: Option<usize> = None;
    for line in log.lines() {
        if let Some(i) = line.find("frame:") {
            let rest = &line[i + 6..];
            let n: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            frame = n.parse().ok();
        } else if let Some(i) = line.find("lavfi.signalstats.YAVG=") {
            let v: f64 = line[i + 23..].trim().parse().unwrap_or(0.0);
            if let Some(f) = frame.take() {
                out.push((f, v));
            }
        }
    }
    out
}

fn ffprobe_value(video: &Path, entries: &str) -> Option<String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            entries,
        ])
        .args(["-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(video)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A video's frame rate and length, from ffprobe.
pub fn probe(video: &Path) -> Result<(f64, f64), String> {
    let rate = ffprobe_value(video, "stream=avg_frame_rate")
        .ok_or_else(|| format!("ffprobe could not read {}", video.display()))?;
    let fps = match rate.split_once('/') {
        Some((a, b)) => a.parse::<f64>().unwrap_or(0.0) / b.parse::<f64>().unwrap_or(1.0),
        None => rate.parse().unwrap_or(0.0),
    };
    let dur = ffprobe_value(video, "stream=duration")
        .and_then(|s| s.lines().next().and_then(|l| l.parse::<f64>().ok()))
        .or_else(|| {
            let out = Command::new("ffprobe")
                .args(["-v", "error", "-show_entries", "format=duration"])
                .args(["-of", "default=noprint_wrappers=1:nokey=1"])
                .arg(video)
                .output()
                .ok()?;
            String::from_utf8_lossy(&out.stdout).trim().parse().ok()
        })
        .ok_or_else(|| format!("ffprobe found no length for {}", video.display()))?;
    if !(fps > 0.0 && fps.is_finite()) {
        return Err(format!(
            "ffprobe found no frame rate for {}",
            video.display()
        ));
    }
    Ok((fps, dur))
}

/// Whether ffmpeg (and ffprobe) are on PATH.
pub fn have_ffmpeg() -> bool {
    let ok = |bin: &str| {
        Command::new(bin)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    ok("ffmpeg") && ok("ffprobe")
}

/// Per-frame motion of a video: `motion[i]` belongs to frame i (time i / fps),
/// measured against the frame before it; frame 0 has none.
pub fn measure(video: &Path) -> Result<(Vec<f64>, f64, f64), String> {
    if !have_ffmpeg() {
        return Err(
            "detect needs ffmpeg and ffprobe on PATH; install ffmpeg and run it again".into(),
        );
    }
    let (fps, duration) = probe(video)?;
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(video)
        .args([
            "-vf",
            "scale=320:-1,tblend=all_mode=difference,signalstats,metadata=print:key=lavfi.signalstats.YAVG",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg failed on {}: {}",
            video.display(),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
        ));
    }
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let pairs = parse_motion_log(&log);
    // tblend's output frame k compares input frames k and k + 1, so it
    // belongs to frame k + 1.
    let frames = pairs.iter().map(|p| p.0 + 2).max().unwrap_or(0);
    let mut motion = vec![0.0; frames];
    for (k, v) in pairs {
        motion[k + 1] = v;
    }
    Ok((motion, fps, duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_metadata_log() {
        let log = "[Parsed_metadata_3 @ 0x1] frame:0    pts:0       pts_time:0\n\
                   [Parsed_metadata_3 @ 0x1] lavfi.signalstats.YAVG=0.25\n\
                   [Parsed_metadata_3 @ 0x1] frame:1    pts:512     pts_time:0.0333\n\
                   [Parsed_metadata_3 @ 0x1] lavfi.signalstats.YAVG=12.5\n";
        assert_eq!(parse_motion_log(log), vec![(0, 0.25), (1, 12.5)]);
    }

    #[test]
    fn spans_roll_over_minutes() {
        assert_eq!(span(65.3, 70.0), "1:05.300-1:10.000");
        assert_eq!(span(0.0, 1.2), "0:00.000-0:01.200");
    }
}
