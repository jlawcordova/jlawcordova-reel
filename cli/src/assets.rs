//! `asset add` (spec §7.12): outside animations, registered by hash.

use crate::files::{AssetAudio, AssetEntry, AssetFit};
use crate::project::{Errors, Project};
use std::path::Path;

pub fn add(
    p: &Project,
    file: &Path,
    id: &str,
    fit: AssetFit,
    alpha: bool,
    audio: AssetAudio,
) -> Result<Vec<String>, Errors> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(vec![format!(
            "asset id \"{id}\" may only use a-z, 0-9, - and _"
        )]);
    }
    if !file.is_file() {
        return Err(vec![format!("{} isn't a file", file.display())]);
    }
    let probe = crate::media::probe(file, "asset add").map_err(|e| vec![e])?;
    if !probe.has_video {
        return Err(vec![format!("{} has no video", file.display())]);
    }
    if alpha && !probe.alpha {
        return Err(vec![format!(
            "{} has no alpha channel; --alpha needs ProRes 4444 or VP9 WebM with alpha",
            file.display()
        )]);
    }
    if probe.duration <= 0.0 {
        return Err(vec![format!("{} has no duration", file.display())]);
    }
    let stored = p.stored_path(file);
    let mut cache = p.hash_cache();
    let sha = cache.file(&p.resolve(&stored)).map_err(|e| vec![e])?;
    cache.save();
    let mut assets = p.assets()?;
    let entry = AssetEntry {
        path: stored.clone(),
        sha256: sha,
        duration: crate::time::to_ms(probe.duration),
        size: format!("{}x{}", probe.width, probe.height),
        fit,
        alpha,
        audio,
        has_audio: probe.has_audio,
    };
    let replaced = assets.insert(id.to_string(), entry).is_some();
    p.save_assets(&assets)?;
    let mut out = vec![format!(
        "{} asset {id}: {stored}, {:.2} s, {}x{}{}{}",
        if replaced { "updated" } else { "added" },
        probe.duration,
        probe.width,
        probe.height,
        if alpha { ", alpha" } else { "" },
        if probe.has_audio && audio == AssetAudio::Keep {
            ", with audio"
        } else {
            ""
        }
    )];
    if crate::cuts::locked(p).is_some() {
        out.push("note: the timeline is locked; if this asset sets a beat's length, run pixelreel cut unlock and cut lock".into());
    }
    Ok(out)
}
