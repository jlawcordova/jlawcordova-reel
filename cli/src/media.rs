//! `ffprobe` and `ffmpeg` helpers.

use crate::proc;
use std::path::Path;
use std::process::Command;

/// The audio sample rate everything is mixed at.
pub const SR: u32 = 48_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub has_video: bool,
    pub has_audio: bool,
    pub alpha: bool,
}

pub fn ffmpeg() -> Command {
    let mut c = Command::new(std::env::var("PIXELREEL_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()));
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    c
}

fn ffprobe() -> Command {
    Command::new(std::env::var("PIXELREEL_FFPROBE").unwrap_or_else(|_| "ffprobe".into()))
}

pub fn probe(path: &Path, purpose: &str) -> Result<Probe, String> {
    let out = proc::run(
        ffprobe()
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(path),
        purpose,
    )?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        format!(
            "{purpose}: ffprobe gave unreadable output for {}: {e}",
            path.display()
        )
    })?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let num = |x: &serde_json::Value| -> Option<f64> {
        x.as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| x.as_f64())
    };
    let duration = num(&v["format"]["duration"])
        .or_else(|| video.and_then(|s| num(&s["duration"])))
        .unwrap_or(0.0);
    let (width, height, alpha) = match video {
        Some(s) => {
            let pix = s["pix_fmt"].as_str().unwrap_or("");
            let codec = s["codec_name"].as_str().unwrap_or("");
            // VP9 WebM keeps alpha in side data, flagged by the alpha_mode tag.
            let webm_alpha = s["tags"]["alpha_mode"].as_str() == Some("1")
                || s["tags"]["ALPHA_MODE"].as_str() == Some("1");
            let a = pix.starts_with("yuva")
                || pix.contains("rgba")
                || pix.contains("argb")
                || pix.contains("bgra")
                || pix.contains("abgr")
                || pix.starts_with("gbrap")
                || pix == "pal8"
                || (codec == "vp9" && webm_alpha)
                || (codec == "vp8" && webm_alpha);
            (
                s["width"].as_u64().unwrap_or(0) as u32,
                s["height"].as_u64().unwrap_or(0) as u32,
                a,
            )
        }
        None => (0, 0, false),
    };
    Ok(Probe {
        duration,
        width,
        height,
        has_video: video.is_some(),
        has_audio,
        alpha,
    })
}

/// Decodes `[start, start + dur)` of a file's audio to stereo f32 at `SR`.
/// `dur` of `None` reads to the end. Missing audio reads as silence.
pub fn decode_audio(
    path: &Path,
    start: f64,
    dur: Option<f64>,
    purpose: &str,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    let mut cmd = ffmpeg();
    if start > 0.0 {
        cmd.args(["-ss", &format!("{start:.6}")]);
    }
    if let Some(d) = dur {
        cmd.args(["-t", &format!("{:.6}", d.max(0.0))]);
    }
    cmd.arg("-i").arg(path).args([
        "-vn",
        "-ac",
        "2",
        "-ar",
        &SR.to_string(),
        "-f",
        "f32le",
        "-c:a",
        "pcm_f32le",
        "pipe:1",
    ]);
    let out = proc::run(&mut cmd, purpose)?;
    let n = out.stdout.len() / 8;
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for ch in out.stdout.chunks_exact(8) {
        l.push(f32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]));
        r.push(f32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]));
    }
    Ok((l, r))
}

/// Writes 16 kHz mono 16-bit audio for whisper.
pub fn extract_for_whisper(input: &Path, out: &Path, purpose: &str) -> Result<(), String> {
    proc::run(
        ffmpeg()
            .arg("-i")
            .arg(input)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
            .arg(out),
        purpose,
    )
    .map(|_| ())
}

/// Converts any audio file to a 16-bit WAV at its own rate, keeping channels.
pub fn to_wav(input: &Path, out: &Path, purpose: &str) -> Result<(), String> {
    proc::run(
        ffmpeg()
            .arg("-i")
            .arg(input)
            .args(["-vn", "-c:a", "pcm_s16le"])
            .arg(out),
        purpose,
    )
    .map(|_| ())
}

/// A font file for `drawtext`, found with `fc-match`.
pub fn font_file() -> Option<String> {
    if let Ok(f) = std::env::var("PIXELREEL_FONT") {
        return Some(f);
    }
    let out = Command::new("fc-match")
        .args(["-f", "%{file}", "sans-serif:style=Bold"])
        .output()
        .ok()?;
    let f = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!f.is_empty() && Path::new(&f).exists()).then_some(f)
}

/// Quotes a path or value for an ffmpeg filter option: inside single quotes
/// only a single quote itself needs escaping.
pub fn filter_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
