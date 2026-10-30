//! ElevenLabs lines (spec §7.5, §7.6): text-to-speech with timestamps,
//! through `curl`, cached by hash.

use crate::align::TWord;
use crate::config::Voice as VoiceCfg;
use crate::project::{Errors, Project};
use crate::storyboard::{Line, Storyboard, Voice};
use crate::timeline::LineAudio;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DEFAULT_URL: &str = "https://api.elevenlabs.io";
const OUTPUT_FORMAT: &str = "mp3_44100_128";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WordsFile {
    pub text: String,
    pub voice: String,
    pub take: u32,
    pub duration: f64,
    pub words: Vec<TWord>,
}

/// The cache key for a line's text in a voice (the take number is the
/// request's seed and goes in the file name).
pub fn line_hash(text: &str, v: &VoiceCfg) -> String {
    let key = json!({
        "text": text, "voice_id": v.voice_id, "model": v.model,
        "stability": v.stability, "similarity": v.similarity,
    });
    crate::hash::sha256(key.to_string().as_bytes())[..16].to_string()
}

pub fn paths(p: &Project, hash: &str, take: u32) -> (PathBuf, PathBuf) {
    let dir = p.vo_dir();
    (
        dir.join(format!("{hash}-{take}.wav")),
        dir.join(format!("{hash}-{take}.words.json")),
    )
}

fn read_audio(p: &Project, hash: &str, take: u32) -> Option<LineAudio> {
    let (wav, words) = paths(p, hash, take);
    if !wav.exists() {
        return None;
    }
    let w: WordsFile = serde_json::from_str(&std::fs::read_to_string(words).ok()?).ok()?;
    Some(LineAudio {
        file: p.stored_path(&wav),
        duration: w.duration,
        words: w.words,
    })
}

/// A VO line's picked take, if it has been generated.
pub fn picked_audio(p: &Project, line: &Line, picks: &crate::files::Picks) -> Option<LineAudio> {
    let Voice::Vo(v) = &line.voice else {
        return None;
    };
    let cfg = p.config.voices.get(v)?;
    let take = picks.vo.get(&line.id).copied().unwrap_or(1);
    read_audio(p, &line_hash(&line.text, cfg), take)
}

/// Draft audio for any line in the animatic's voice (`animatic --tts`).
pub fn draft_audio(p: &Project, line: &Line, voice: &str) -> Option<LineAudio> {
    let cfg = p.config.voices.get(voice)?;
    read_audio(p, &line_hash(&line.text, cfg), 1)
}

/// Words with their times from ElevenLabs' character alignment.
pub fn words_from_alignment(alignment: &Value) -> Vec<TWord> {
    let chars = alignment["characters"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let starts = alignment["character_start_times_seconds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let ends = alignment["character_end_times_seconds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut words = Vec::new();
    let mut cur: Option<TWord> = None;
    for (i, ch) in chars.iter().enumerate() {
        let c = ch.as_str().unwrap_or(" ");
        let s = starts.get(i).and_then(Value::as_f64).unwrap_or(0.0);
        let e = ends.get(i).and_then(Value::as_f64).unwrap_or(s);
        if c.chars().all(char::is_whitespace) {
            if let Some(w) = cur.take() {
                words.push(w);
            }
            continue;
        }
        match cur.as_mut() {
            Some(w) => {
                w.w.push_str(c);
                w.end = w.end.max(e);
            }
            None => {
                cur = Some(TWord {
                    w: c.to_string(),
                    start: s,
                    end: e,
                })
            }
        }
    }
    if let Some(w) = cur {
        words.push(w);
    }
    words
}

/// One request: returns the audio bytes and the words.
fn request(
    base: &str,
    key: &str,
    v: &VoiceCfg,
    text: &str,
    seed: u32,
    tmp: &Path,
) -> Result<(Vec<u8>, Vec<TWord>), String> {
    let url = format!(
        "{}/v1/text-to-speech/{}/with-timestamps?output_format={OUTPUT_FORMAT}",
        base.trim_end_matches('/'),
        v.voice_id
    );
    let body = json!({
        "text": text,
        "model_id": v.model,
        "voice_settings": { "stability": v.stability, "similarity_boost": v.similarity },
        "seed": seed,
    });
    std::fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
    let body_path = tmp.join("request.json");
    let resp_path = tmp.join("response.json");
    std::fs::write(&body_path, body.to_string()).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&resp_path);
    // The key goes in on stdin (-H @-), never on the command line.
    let out = crate::proc::run_with_input(
        Command::new("curl")
            .args([
                "-sS",
                "-X",
                "POST",
                "-H",
                "@-",
                "-H",
                "Content-Type: application/json",
            ])
            .arg("--data-binary")
            .arg(format!("@{}", body_path.display()))
            .arg("-o")
            .arg(&resp_path)
            .args(["-w", "%{http_code}"])
            .arg(&url),
        "vo gen",
        Some(format!("xi-api-key: {key}\n").as_bytes()),
    )?;
    let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let resp = std::fs::read(&resp_path).unwrap_or_default();
    let _ = std::fs::remove_file(&body_path);
    let _ = std::fs::remove_file(&resp_path);
    if code != "200" {
        let msg = serde_json::from_slice::<Value>(&resp)
            .ok()
            .and_then(|v| {
                v["detail"]["message"]
                    .as_str()
                    .or_else(|| v["detail"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&resp).chars().take(300).collect());
        return Err(format!("ElevenLabs answered HTTP {code}: {msg}"));
    }
    let v: Value = serde_json::from_slice(&resp)
        .map_err(|e| format!("ElevenLabs sent unreadable JSON: {e}"))?;
    let audio = base64::engine::general_purpose::STANDARD
        .decode(v["audio_base64"].as_str().unwrap_or(""))
        .map_err(|e| format!("ElevenLabs sent unreadable audio: {e}"))?;
    if audio.is_empty() {
        return Err("ElevenLabs sent no audio".into());
    }
    let alignment = if v["alignment"].is_object() {
        &v["alignment"]
    } else {
        &v["normalized_alignment"]
    };
    Ok((audio, words_from_alignment(alignment)))
}

/// `vo gen` (spec §7.5). Returns lines to print and warnings.
pub fn generate(
    p: &Project,
    sb: &Storyboard,
    beats: &[String],
    voice: Option<&str>,
    takes: u32,
    force: bool,
) -> Result<(Vec<String>, Vec<String>), Errors> {
    let mut errors = Vec::new();
    for b in beats {
        if !sb.beats.iter().any(|x| &x.id == b) {
            errors.push(format!("no beat \"{b}\" in storyboard.md"));
        }
    }
    if let Some(v) = voice {
        if !p.config.voices.contains_key(v) {
            errors.push(format!("no voice \"{v}\" in pixelreel.yaml"));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let lines: Vec<(&Line, String)> = sb
        .beats
        .iter()
        .filter(|b| beats.is_empty() || beats.contains(&b.id))
        .flat_map(|b| b.lines.iter())
        .filter_map(|l| match &l.voice {
            Voice::Vo(v) if voice.is_none_or(|x| x == v) => Some((l, v.clone())),
            _ => None,
        })
        .collect();
    synthesize(p, &lines, takes, force, "vo gen")
}

/// Generates audio for each (line, voice), `takes` takes each.
pub fn synthesize(
    p: &Project,
    lines: &[(&Line, String)],
    takes: u32,
    force: bool,
    purpose: &str,
) -> Result<(Vec<String>, Vec<String>), Errors> {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut todo = Vec::new();
    for (l, v) in lines {
        let cfg = &p.config.voices[v];
        let hash = line_hash(&l.text, cfg);
        for take in 1..=takes.max(1) {
            let (wav, _) = paths(p, &hash, take);
            if wav.exists() && !force {
                continue;
            }
            todo.push((*l, v.clone(), hash.clone(), take));
        }
    }
    if todo.is_empty() {
        out.push(format!(
            "{} line(s) already generated; nothing to do",
            lines.len()
        ));
        return Ok((out, warnings));
    }
    let key = std::env::var("ELEVENLABS_API_KEY").map_err(|_| {
        vec![format!(
            "{purpose} needs the ELEVENLABS_API_KEY environment variable ({} request(s) to make)",
            todo.len()
        )]
    })?;
    let base =
        std::env::var("PIXELREEL_ELEVENLABS_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let locked = crate::timeline::load(&p.timeline_path()).filter(|t| t.locked);
    let tmp = p.build().join("vo-tmp");
    let mut errors = Vec::new();
    for (l, v, hash, take) in todo {
        let cfg = &p.config.voices[&v];
        match request(&base, &key, cfg, &l.text, take, &tmp) {
            Ok((audio, words)) => {
                let (wav, words_path) = paths(p, &hash, take);
                let mp3 = tmp.join(format!("{hash}-{take}.mp3"));
                let saved = std::fs::write(&mp3, &audio)
                    .map_err(|e| e.to_string())
                    .and_then(|_| crate::media::to_wav(&mp3, &wav, purpose))
                    .and_then(|_| crate::media::probe(&wav, purpose));
                let _ = std::fs::remove_file(&mp3);
                match saved {
                    Ok(probe) => {
                        let wf = WordsFile {
                            text: l.text.clone(),
                            voice: v.clone(),
                            take,
                            duration: probe.duration,
                            words,
                        };
                        let text = serde_json::to_string_pretty(&wf).unwrap_or_default();
                        if let Err(e) =
                            crate::files::write_atomic(&words_path, format!("{text}\n").as_bytes())
                        {
                            errors.extend(e);
                            continue;
                        }
                        out.push(format!(
                            "{} take {take}: {:.2} s -> {}",
                            l.id,
                            probe.duration,
                            p.stored_path(&wav)
                        ));
                        if locked
                            .as_ref()
                            .is_some_and(|t| t.lines().any(|x| x.id == l.id))
                        {
                            warnings.push(format!(
                                "{} is in the locked timeline; run pixelreel cut unlock and cut lock to use the new audio",
                                l.id
                            ));
                        }
                    }
                    Err(e) => errors.push(format!("{}: {e}", l.id)),
                }
            }
            Err(e) => errors.push(format!("{} take {take}: {e}", l.id)),
        }
    }
    let _ = std::fs::remove_dir(&tmp);
    if errors.is_empty() {
        Ok((out, warnings))
    } else {
        Err(errors)
    }
}

/// `vo pick` (spec §7.6): a line, or every VO line in a beat.
pub fn pick(p: &Project, sb: &Storyboard, target: &str, take: u32) -> Result<Vec<String>, Errors> {
    let lines: Vec<&Line> = if target.contains('.') {
        sb.beats
            .iter()
            .flat_map(|b| b.lines.iter())
            .filter(|l| l.id == target)
            .collect()
    } else {
        sb.beats
            .iter()
            .filter(|b| b.id == target)
            .flat_map(|b| b.lines.iter())
            .filter(|l| matches!(l.voice, Voice::Vo(_)))
            .collect()
    };
    if lines.is_empty() {
        return Err(vec![format!(
            "no VO line or beat with VO lines called \"{target}\""
        )]);
    }
    let mut picks = p.picks()?;
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for l in lines {
        let Voice::Vo(v) = &l.voice else {
            errors.push(format!("{} is your line, not a VO line", l.id));
            continue;
        };
        let cfg = &p.config.voices[v];
        let (wav, _) = paths(p, &line_hash(&l.text, cfg), take);
        if !wav.exists() {
            errors.push(format!(
                "{} has no take {take}; run pixelreel vo gen --takes {take}",
                l.id
            ));
            continue;
        }
        picks.vo.insert(l.id.clone(), take);
        out.push(format!("{} -> take {take}", l.id));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    p.save_picks(&picks)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_characters_into_words() {
        let a = json!({
            "characters": ["H","i",","," ","y","o","u","."],
            "character_start_times_seconds": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
            "character_end_times_seconds":   [0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9]
        });
        let w = words_from_alignment(&a);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].w, "Hi,");
        assert_eq!((w[0].start, w[0].end), (0.1, 0.4));
        assert_eq!(w[1].w, "you.");
        assert_eq!((w[1].start, w[1].end), (0.5, 0.9));
    }

    #[test]
    fn hash_follows_text_and_settings() {
        let v = VoiceCfg {
            voice_id: "a".into(),
            model: "m".into(),
            stability: 0.5,
            similarity: 0.75,
        };
        let mut v2 = v.clone();
        v2.stability = 0.6;
        assert_eq!(line_hash("Hi.", &v), line_hash("Hi.", &v));
        assert_ne!(line_hash("Hi.", &v), line_hash("Hi!", &v));
        assert_ne!(line_hash("Hi.", &v), line_hash("Hi.", &v2));
    }
}
