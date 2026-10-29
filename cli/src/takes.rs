//! `take add` (spec §7.7): transcribe recordings with whisper.cpp and find
//! each line's reads in them.

use crate::align::{self, Query, Read, TWord, TakeAlignment, Transcript};
use crate::files::TakeEntry;
use crate::project::{Errors, Project};
use crate::storyboard::{Storyboard, Voice};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn transcript_path(p: &Project, name: &str) -> PathBuf {
    p.takes_build().join(format!("{name}.transcript.json"))
}

pub fn align_path(p: &Project, name: &str) -> PathBuf {
    p.takes_build().join(format!("{name}.align.json"))
}

/// Words from whisper.cpp's `-oj` output made with `-ml 1 -sow`, where each
/// segment is one word with `offsets` in ms.
pub fn parse_whisper(v: &Value) -> Vec<TWord> {
    let mut out = Vec::new();
    for seg in v["transcription"].as_array().into_iter().flatten() {
        let text = seg["text"].as_str().unwrap_or("").trim();
        if text.is_empty() || (text.starts_with('[') && text.ends_with(']')) {
            continue;
        }
        let from = seg["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0;
        let to = seg["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0;
        // A segment can still hold several words; share its time evenly.
        let words: Vec<&str> = text.split_whitespace().collect();
        let slot = (to - from).max(0.0) / words.len() as f64;
        for (i, w) in words.iter().enumerate() {
            if w.chars().all(|c| !c.is_alphanumeric()) {
                // Punctuation on its own joins the word before it.
                if let Some(last) = out.last_mut() {
                    let last: &mut TWord = last;
                    last.w.push_str(w);
                }
                continue;
            }
            out.push(TWord {
                w: w.to_string(),
                start: from + i as f64 * slot,
                end: from + (i + 1) as f64 * slot,
            });
        }
    }
    out
}

fn transcribe(p: &Project, file: &Path, name: &str, sha: &str) -> Result<Transcript, String> {
    let model = p.config.transcribe.model.as_deref().ok_or_else(|| {
        "take add needs transcribe.model in pixelreel.yaml (a whisper.cpp model, like ~/models/ggml-base.en.bin)".to_string()
    })?;
    let model = p.resolve(model);
    let dir = p.takes_build();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wav = dir.join(format!("{name}.16k.wav"));
    crate::media::extract_for_whisper(file, &wav, "take add")?;
    let base = dir.join(format!("{name}.whisper"));
    let json_path = base.with_extension("whisper.json");
    let _ = std::fs::remove_file(&json_path);
    crate::proc::run(
        Command::new(&p.config.transcribe.bin)
            .arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(&wav)
            .args(["-oj", "-ml", "1", "-sow", "-np", "-of"])
            .arg(&base),
        "take add",
    )?;
    let text = std::fs::read_to_string(&json_path).map_err(|e| {
        format!(
            "take add: {} wrote no {}: {e}",
            p.config.transcribe.bin,
            json_path.display()
        )
    })?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| format!("take add: unreadable whisper JSON: {e}"))?;
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&json_path);
    Ok(Transcript {
        sha256: sha.to_string(),
        words: parse_whisper(&v),
    })
}

fn load_transcript(p: &Project, name: &str) -> Option<Transcript> {
    let text = std::fs::read_to_string(transcript_path(p, name)).ok()?;
    serde_json::from_str(&text).ok()
}

/// `take add`. Returns lines to print and warnings.
pub fn add(p: &Project, files: &[PathBuf]) -> Result<(Vec<String>, Vec<String>), Errors> {
    let sb = p.storyboard()?;
    let mut takes = p.takes()?;
    let mut cache = p.hash_cache();
    let mut errors = Vec::new();
    let mut out = Vec::new();
    for f in files {
        if !f.is_file() {
            errors.push(format!("{} isn't a file", f.display()));
            continue;
        }
        let name = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stored = p.stored_path(f);
        if let Some(other) = takes
            .takes
            .iter()
            .find(|t| t.name == name && t.path != stored)
        {
            errors.push(format!(
                "a take called {name} is already added from {}; rename {} so its name is unique",
                other.path,
                f.display()
            ));
            continue;
        }
        let probe = match crate::media::probe(f, "take add") {
            Ok(pr) if pr.has_audio && pr.has_video => pr,
            Ok(_) => {
                errors.push(format!("{} needs both video and audio", f.display()));
                continue;
            }
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let sha = match cache.file(&p.resolve(&stored)) {
            Ok(h) => h,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let tr = match load_transcript(p, &name).filter(|t| t.sha256 == sha) {
            Some(t) => t,
            None => match transcribe(p, f, &name, &sha) {
                Ok(t) => {
                    let text = serde_json::to_string_pretty(&t).unwrap_or_default();
                    if let Err(e) =
                        crate::files::write_atomic(&transcript_path(p, &name), text.as_bytes())
                    {
                        errors.extend(e);
                        continue;
                    }
                    t
                }
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            },
        };
        out.push(format!(
            "{name}: {:.1} s, {} words transcribed",
            probe.duration,
            tr.words.len()
        ));
        let entry = TakeEntry {
            name: name.clone(),
            path: stored,
            sha256: sha,
            duration: probe.duration,
        };
        match takes.takes.iter_mut().find(|t| t.name == name) {
            Some(t) => *t = entry,
            None => takes.takes.push(entry),
        }
    }
    cache.save();
    if !errors.is_empty() {
        // Keep what did work.
        p.save_takes(&takes)?;
        return Err(errors);
    }
    p.save_takes(&takes)?;
    let (reads, aligns, mut warnings) = reads_by_line(p, &sb)?;
    for a in &aligns {
        let text = serde_json::to_string_pretty(a).unwrap_or_default();
        crate::files::write_atomic(&align_path(p, &a.take), text.as_bytes())?;
    }
    out.extend(report(&sb, &reads, &aligns));
    let missing: Vec<String> = sb
        .beats
        .iter()
        .flat_map(|b| b.lines.iter())
        .filter(|l| l.voice == Voice::You)
        .filter(|l| {
            !reads
                .get(&l.id)
                .is_some_and(|rs| rs.iter().any(|r| r.complete))
        })
        .map(|l| l.id.clone())
        .collect();
    if !missing.is_empty() {
        warnings.push(format!(
            "lines with no complete read yet: {}",
            missing.join(", ")
        ));
    }
    Ok((out, warnings))
}

fn report(
    sb: &Storyboard,
    reads: &BTreeMap<String, Vec<Read>>,
    aligns: &[TakeAlignment],
) -> Vec<String> {
    let mut out = Vec::new();
    for b in &sb.beats {
        for l in b.lines.iter().filter(|l| l.voice == Voice::You) {
            let rs = reads.get(&l.id).map(Vec::as_slice).unwrap_or(&[]);
            if rs.is_empty() {
                continue;
            }
            let complete = rs.iter().filter(|r| r.complete).count();
            let last = rs.iter().rfind(|r| r.complete);
            out.push(format!(
                "  {:<14} {} read(s), {} complete{}",
                l.id,
                rs.len(),
                complete,
                last.map_or(String::new(), |r| format!(
                    "; using {} {}–{}",
                    r.take,
                    crate::time::clock(r.start),
                    crate::time::clock(r.end)
                ))
            ));
        }
    }
    for a in aligns {
        for (s, e) in &a.unmatched {
            out.push(format!(
                "  ad-lib in {} at {}–{} (not in the script)",
                a.take,
                crate::time::clock(*s),
                crate::time::clock(*e)
            ));
        }
    }
    out
}

/// Reads per line, per-take alignments, and warnings.
pub type Aligned = (BTreeMap<String, Vec<Read>>, Vec<TakeAlignment>, Vec<String>);

/// Aligns every take's transcript against the storyboard's you-lines, in
/// takes.yaml order. Returns reads per line (take order, then time), the
/// per-take alignments, and warnings.
pub fn reads_by_line(p: &Project, sb: &Storyboard) -> Result<Aligned, Errors> {
    let takes = p.takes()?;
    let queries: Vec<Query> = sb
        .beats
        .iter()
        .flat_map(|b| b.lines.iter())
        .filter(|l| l.voice == Voice::You)
        .map(|l| Query {
            id: &l.id,
            words: &l.words,
        })
        .collect();
    let mut by_line: BTreeMap<String, Vec<Read>> = BTreeMap::new();
    let mut aligns = Vec::new();
    let mut errors = Vec::new();
    for t in &takes.takes {
        let Some(tr) = load_transcript(p, &t.name) else {
            errors.push(format!(
                "take {} has no transcript in build/takes; run pixelreel take add {}",
                t.name, t.path
            ));
            continue;
        };
        if tr.sha256 != t.sha256 {
            errors.push(format!(
                "take {}'s transcript is of an older recording; run pixelreel take add {}",
                t.name, t.path
            ));
            continue;
        }
        let a = align::align_take(&t.name, &queries, &tr);
        for r in &a.reads {
            by_line.entry(r.line.clone()).or_default().push(r.clone());
        }
        aligns.push(a);
    }
    if errors.is_empty() {
        Ok((by_line, aligns, Vec::new()))
    } else {
        Err(errors)
    }
}

/// Take name → (stored path, duration), for the lock.
pub fn take_files(p: &Project) -> Result<BTreeMap<String, (String, f64)>, Errors> {
    Ok(p.takes()?
        .takes
        .into_iter()
        .map(|t| (t.name, (t.path, t.duration)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_whisper_word_segments() {
        let v = json!({"transcription": [
            {"offsets": {"from": 0, "to": 320}, "text": " Thirty"},
            {"offsets": {"from": 320, "to": 600}, "text": " days"},
            {"offsets": {"from": 600, "to": 610}, "text": "."},
            {"offsets": {"from": 700, "to": 900}, "text": " [BLANK_AUDIO]"},
            {"offsets": {"from": 1000, "to": 1400}, "text": " big farm"}
        ]});
        let w = parse_whisper(&v);
        assert_eq!(w.len(), 4);
        assert_eq!(w[1].w, "days.");
        assert_eq!((w[0].start, w[0].end), (0.0, 0.32));
        assert_eq!(w[2].w, "big");
        assert!((w[3].start - 1.2).abs() < 1e-9);
    }
}
