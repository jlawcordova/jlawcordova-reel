//! The decision files next to the storyboard: `cuts.yaml`, `picks.yaml`,
//! `takes.yaml` and `assets/assets.yaml` (spec §5, §6.3, §6.4, §6.7).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cuts {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lines: BTreeMap<String, LineCut>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub beats: BTreeMap<String, BeatCut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tighten: Option<Tighten>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineCut {
    /// Seconds added to the aligned in point.
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub in_: Option<f64>,
    /// Seconds added to the aligned out point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<f64>,
    /// Use this take's last complete read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<String>,
}

impl LineCut {
    pub fn is_empty(&self) -> bool {
        self.in_.is_none() && self.out.is_none() && self.take.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeatCut {
    /// The picture starts this many seconds after the voice (a J-cut).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<f64>,
    /// Seconds added to (or taken off) the end of the beat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
}

impl BeatCut {
    pub fn is_empty(&self) -> bool {
        self.lead.is_none() && self.length.is_none()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tighten {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_breath: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picks {
    /// VO take number by line id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vo: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ScorePick>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScorePick {
    pub seed: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Takes {
    #[serde(default)]
    pub takes: Vec<TakeEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeEntry {
    /// The file stem, like `A001`.
    pub name: String,
    pub path: String,
    pub sha256: String,
    pub duration: f64,
}

pub type Assets = BTreeMap<String, AssetEntry>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AssetFit {
    Trim,
    HoldLast,
    Speed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AssetAudio {
    Keep,
    Mute,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetEntry {
    pub path: String,
    pub sha256: String,
    pub duration: f64,
    pub size: String,
    pub fit: AssetFit,
    pub alpha: bool,
    pub audio: AssetAudio,
    #[serde(default)]
    pub has_audio: bool,
}

/// Reads a YAML file, or the default when it doesn't exist or is empty.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(T::default()),
        Ok(t) => serde_yaml::from_str(&t).map_err(|e| vec![format!("{}: {e}", path.display())]),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(vec![format!("can't read {}: {e}", path.display())]),
    }
}

/// Writes a YAML file atomically.
pub fn save<T: Serialize>(path: &Path, value: &T, header: &str) -> Result<(), Vec<String>> {
    let body = serde_yaml::to_string(value).map_err(|e| vec![e.to_string()])?;
    let body = if body.trim() == "{}" {
        String::new()
    } else {
        body
    };
    write_atomic(path, format!("{header}{body}").as_bytes())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Vec<String>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| vec![format!("can't create {}: {e}", dir.display())])?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, bytes).map_err(|e| vec![format!("can't write {}: {e}", tmp.display())])?;
    std::fs::rename(&tmp, path).map_err(|e| vec![format!("can't write {}: {e}", path.display())])
}

pub const CUTS_HEADER: &str =
    "# Timing edits, written by `pixelreel cut` (spec §6.3). Safe to edit by hand.\n";
pub const PICKS_HEADER: &str = "# Chosen VO takes and music seed (spec §6.4).\n";
pub const TAKES_HEADER: &str = "# Recordings added with `pixelreel take add`, in order.\n";
pub const ASSETS_HEADER: &str = "# Outside animations registered with `pixelreel asset add`.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_round_trip_and_reject_unknown_keys() {
        let text = "lines:\n  hook.2: { out: -0.2 }\n  intro.1: { take: A003, in: 0.1 }\nbeats:\n  hook-tease: { lead: 0.4, length: 0.21 }\ntighten: { max_gap: 0.3, keep_breath: 0.12 }\n";
        let c: Cuts = serde_yaml::from_str(text).unwrap();
        assert_eq!(c.lines["hook.2"].out, Some(-0.2));
        assert_eq!(c.lines["intro.1"].in_, Some(0.1));
        assert_eq!(c.beats["hook-tease"].lead, Some(0.4));
        let back: Cuts = serde_yaml::from_str(&serde_yaml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert!(serde_yaml::from_str::<Cuts>("lines:\n  a.1: { outt: 1 }\n").is_err());
    }

    #[test]
    fn missing_files_are_defaults() {
        let p = std::env::temp_dir().join("pixelreel-no-such-file.yaml");
        let c: Cuts = load(&p).unwrap();
        assert_eq!(c, Cuts::default());
    }
}
