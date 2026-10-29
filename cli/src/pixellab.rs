//! The Pixel Lab adapter (spec §6.6, §9.1): scenes and sequences in a
//! `.pixellab` document, rendered with `pixellab art export`.

use crate::proc;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct PixelLab {
    pub bin: String,
    pub document: PathBuf,
}

/// A scene beat's export: what Pixel Lab is asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    /// A scene name, or `sequences/<name>`.
    pub name: String,
    pub from_ms: u64,
    pub to_ms: u64,
    pub entrance: bool,
    pub fit: (u32, u32),
    pub fps: u32,
    pub background: String,
    pub theme: String,
}

/// A cue key: at `time_ms` on the scene's clock, `item` enters `state`.
#[derive(Clone, Debug, PartialEq)]
pub struct CueKey {
    pub item: String,
    pub state: String,
    pub time_ms: u64,
}

pub fn is_sequence(name: &str) -> bool {
    name.starts_with("sequences/")
}

impl Export {
    /// The arguments after `pixellab`, without `--document`.
    pub fn args(&self, out_dir: &Path) -> Vec<String> {
        let mut a = vec![
            "art".to_string(),
            "export".to_string(),
            self.name.clone(),
            out_dir.to_string_lossy().into_owned(),
            "--format".into(),
            "frames".into(),
            "--fit".into(),
            format!("{},{}", self.fit.0, self.fit.1),
            "--fps".into(),
            self.fps.to_string(),
            "--from".into(),
            self.from_ms.to_string(),
            "--to".into(),
            self.to_ms.to_string(),
            "--background".into(),
            self.background.clone(),
            "--theme".into(),
            self.theme.clone(),
        ];
        if self.entrance {
            a.push("--entrance".into());
        }
        a
    }
}

impl PixelLab {
    fn cmd(&self, document: &Path) -> Command {
        let mut c = Command::new(&self.bin);
        c.arg("--json").arg("--document").arg(document);
        c
    }

    fn json(&self, document: &Path, args: &[&str], purpose: &str) -> Result<Value, String> {
        let out = proc::run(self.cmd(document).args(args), purpose)?;
        serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("{purpose}: {} gave unreadable JSON: {e}", self.bin))
    }

    /// Scene names and sequence names (as `sequences/<name>`).
    pub fn list(&self, purpose: &str) -> Result<(Vec<String>, Vec<String>), String> {
        let names = |v: &Value, key: &str| -> Vec<String> {
            v[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    s["name"]
                        .as_str()
                        .or_else(|| s.as_str())
                        .map(str::to_string)
                })
                .collect()
        };
        let scenes = self.json(&self.document, &["scene", "list"], purpose)?;
        let seqs = self.json(&self.document, &["sequence", "list"], purpose)?;
        Ok((
            names(&scenes, "scenes"),
            names(&seqs, "sequences")
                .into_iter()
                .map(|n| format!("sequences/{n}"))
                .collect(),
        ))
    }

    /// One loop of a scene (`scene show`'s period), or a sequence's length,
    /// in ms.
    pub fn period_ms(&self, document: &Path, name: &str, purpose: &str) -> Result<f64, String> {
        if let Some(seq) = name.strip_prefix("sequences/") {
            let v = self.json(document, &["sequence", "show", seq], purpose)?;
            v["length"]
                .as_f64()
                .ok_or_else(|| format!("{purpose}: sequence show {seq} gave no length"))
        } else {
            let v = self.json(document, &["scene", "show", name], purpose)?;
            Ok(v["period"].as_f64().unwrap_or(1000.0))
        }
    }

    /// Copies the document, lengthens the scene's motion loop to cover
    /// `end_ms` if it's shorter, and keys each cue's item into its state.
    /// The owner's document is never written. Returns warnings.
    pub fn cue_copy(
        &self,
        name: &str,
        keys: &[CueKey],
        end_ms: u64,
        dest: &Path,
        purpose: &str,
    ) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if dest.exists() {
            std::fs::remove_dir_all(dest)
                .map_err(|e| format!("can't clear {}: {e}", dest.display()))?;
        }
        proc::run(
            self.cmd(&self.document)
                .args(["document", "copy", "--to"])
                .arg(dest),
            purpose,
        )?;
        let period = self.period_ms(dest, name, purpose)?;
        let needed = keys
            .iter()
            .map(|k| k.time_ms + 1)
            .max()
            .unwrap_or(0)
            .max(end_ms);
        if (needed as f64) > period {
            proc::run(
                self.cmd(dest)
                    .args(["scene", "motion", "set", name, "--duration"])
                    .arg(needed.to_string()),
                purpose,
            )?;
            warnings.push(format!(
                "scene {name}: its motion loop ({:.0} ms) is lengthened to {needed} ms for the cues, so its motion plays once over the beat",
                period
            ));
        }
        for k in keys {
            proc::run(
                self.cmd(dest)
                    .args(["scene", "key", "set", name, &k.item, "--time"])
                    .arg(k.time_ms.to_string())
                    .args(["--state", &k.state]),
                purpose,
            )?;
        }
        Ok(warnings)
    }

    /// Exports PNG frames into `out_dir` (made fresh) and returns the frame
    /// files in order.
    pub fn export_frames(
        &self,
        document: &Path,
        e: &Export,
        out_dir: &Path,
        purpose: &str,
    ) -> Result<Vec<PathBuf>, String> {
        if out_dir.exists() {
            std::fs::remove_dir_all(out_dir)
                .map_err(|err| format!("can't clear {}: {err}", out_dir.display()))?;
        }
        if let Some(parent) = out_dir.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        proc::run(self.cmd(document).args(e.args(out_dir)), purpose)?;
        let mut frames: Vec<PathBuf> = std::fs::read_dir(out_dir)
            .map_err(|err| {
                format!(
                    "{purpose}: {} wrote no frames to {}: {err}",
                    self.bin,
                    out_dir.display()
                )
            })?
            .filter_map(|d| d.ok().map(|d| d.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "png"))
            .collect();
        frames.sort_by_key(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        });
        if frames.is_empty() {
            return Err(format!(
                "{purpose}: {} wrote no frames for {}",
                self.bin, e.name
            ));
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_arguments_follow_the_scene_settings() {
        let e = Export {
            name: "farm-night".into(),
            from_ms: 1500,
            to_ms: 9500,
            entrance: true,
            fit: (1920, 1080),
            fps: 30,
            background: "card".into(),
            theme: "dark".into(),
        };
        assert_eq!(
            e.args(Path::new("/tmp/f")).join(" "),
            "art export farm-night /tmp/f --format frames --fit 1920,1080 --fps 30 --from 1500 --to 9500 --background card --theme dark --entrance"
        );
        assert!(is_sequence("sequences/tour"));
        assert!(!is_sequence("tour"));
    }
}
