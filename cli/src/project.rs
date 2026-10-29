//! The project folder (spec §5) and `init` (spec §7.2).

use crate::config::Config;
use crate::files::{self, Assets, Cuts, Picks, Takes};
use crate::storyboard::{self, Storyboard};
use std::path::{Path, PathBuf};

pub struct Project {
    pub root: PathBuf,
    pub config: Config,
}

pub type Errors = Vec<String>;

impl Project {
    pub fn open(dir: &Path) -> Result<Project, Errors> {
        let root = dir
            .canonicalize()
            .map_err(|e| vec![format!("can't open project {}: {e}", dir.display())])?;
        let cfg_path = root.join("pixelreel.yaml");
        let text = std::fs::read_to_string(&cfg_path).map_err(|_| {
            vec![format!(
                "{} has no pixelreel.yaml; run `pixelreel init <dir>` or pass -C <project>",
                root.display()
            )]
        })?;
        let config = Config::parse(&text)?;
        Ok(Project { root, config })
    }

    pub fn storyboard_path(&self) -> PathBuf {
        self.root.join("storyboard.md")
    }
    pub fn cuts_path(&self) -> PathBuf {
        self.root.join("cuts.yaml")
    }
    pub fn picks_path(&self) -> PathBuf {
        self.root.join("picks.yaml")
    }
    pub fn takes_path(&self) -> PathBuf {
        self.root.join("takes.yaml")
    }
    pub fn assets_path(&self) -> PathBuf {
        self.root.join("assets").join("assets.yaml")
    }
    pub fn vo_dir(&self) -> PathBuf {
        self.root.join("vo")
    }
    pub fn build(&self) -> PathBuf {
        self.root.join("build")
    }
    pub fn timeline_path(&self) -> PathBuf {
        self.build().join("timeline.json")
    }
    pub fn score_dir(&self) -> PathBuf {
        self.build().join("score")
    }
    pub fn takes_build(&self) -> PathBuf {
        self.build().join("takes")
    }
    pub fn render_dir(&self, mode: &str) -> PathBuf {
        self.build().join("render").join(mode)
    }

    pub fn storyboard(&self) -> Result<Storyboard, Errors> {
        let text = std::fs::read_to_string(self.storyboard_path())
            .map_err(|e| vec![format!("can't read storyboard.md: {e}")])?;
        let voices: Vec<String> = self.config.voices.keys().cloned().collect();
        storyboard::parse(&text, &voices)
    }

    pub fn storyboard_text(&self) -> String {
        std::fs::read_to_string(self.storyboard_path()).unwrap_or_default()
    }

    pub fn cuts(&self) -> Result<Cuts, Errors> {
        files::load(&self.cuts_path())
    }
    pub fn picks(&self) -> Result<Picks, Errors> {
        files::load(&self.picks_path())
    }
    pub fn takes(&self) -> Result<Takes, Errors> {
        files::load(&self.takes_path())
    }
    pub fn assets(&self) -> Result<Assets, Errors> {
        files::load(&self.assets_path())
    }

    pub fn save_cuts(&self, c: &Cuts) -> Result<(), Errors> {
        files::save(&self.cuts_path(), c, files::CUTS_HEADER)
    }
    pub fn save_picks(&self, p: &Picks) -> Result<(), Errors> {
        files::save(&self.picks_path(), p, files::PICKS_HEADER)
    }
    pub fn save_takes(&self, t: &Takes) -> Result<(), Errors> {
        files::save(&self.takes_path(), t, files::TAKES_HEADER)
    }
    pub fn save_assets(&self, a: &Assets) -> Result<(), Errors> {
        files::save(&self.assets_path(), a, files::ASSETS_HEADER)
    }

    /// A path from a settings or decision file: `~` expands to $HOME and a
    /// relative path is taken from the project folder.
    pub fn resolve(&self, p: &str) -> PathBuf {
        expand(&self.root, p)
    }

    /// The path to store for a file the user named on the command line:
    /// relative to the project when it's inside it.
    pub fn stored_path(&self, p: &Path) -> String {
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|d| d.join(p))
                .unwrap_or_else(|_| p.to_path_buf())
        };
        let abs = abs.canonicalize().unwrap_or(abs);
        match abs.strip_prefix(&self.root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => abs.to_string_lossy().into_owned(),
        }
    }

    pub fn pixellab_document(&self) -> Option<PathBuf> {
        self.config
            .pixellab
            .document
            .as_deref()
            .map(|d| self.resolve(d))
    }

    pub fn hash_cache(&self) -> crate::hash::HashCache {
        crate::hash::HashCache::open(self.build().join("cache").join("hashes.json"))
    }
}

pub fn expand(root: &Path, p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(p);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

const STORYBOARD_TEMPLATE: &str = "\
---
title: My video
---

<!--
Each beat starts with a heading: ## <id> | <type> | <picture>
  type:    a cozy-score video type (cold-open-hook, title-sting, intro,
           explain, visual-demo, build, reveal, reflect, sponsor,
           recap-cta, outro-end-screen)
  picture: face, card, scene:<pixel-lab-scene>, asset:<id>
Directives come right after the heading:
  chapter: <title>    length: 4s    under: face|scene:<name>
  fit: loop|hold      from: 2s      entrance: true
  cue <name>: <item> <state>
Plain lines are what you say. VO(<voice>): lines are generated with ElevenLabs.
Mark a word with {#name} to make a cue.
-->

## hook | cold-open-hook | face
chapter: Start here
What if this video made itself?

## title | title-sting | card
length: 4s

## outro | outro-end-screen | card
length: 20s
";

/// Creates a new project folder (spec §7.2).
pub fn init(
    dir: &Path,
    size: &str,
    fps: u32,
    feel: &str,
    document: Option<&str>,
) -> Result<(), Errors> {
    if dir.exists() {
        let mut rd = std::fs::read_dir(dir)
            .map_err(|e| vec![format!("can't read {}: {e}", dir.display())])?;
        if rd.next().is_some() {
            return Err(vec![format!(
                "{} isn't empty; init only writes into a new or empty folder",
                dir.display()
            )]);
        }
    }
    let text = Config::template(size, fps, feel, document);
    Config::parse(&text)?;
    let w = |p: PathBuf, body: &str| files::write_atomic(&p, body.as_bytes());
    w(dir.join("pixelreel.yaml"), &text)?;
    w(dir.join("storyboard.md"), STORYBOARD_TEMPLATE)?;
    w(dir.join("cuts.yaml"), files::CUTS_HEADER)?;
    w(dir.join("picks.yaml"), files::PICKS_HEADER)?;
    w(dir.join("takes.yaml"), files::TAKES_HEADER)?;
    w(dir.join("assets").join("assets.yaml"), files::ASSETS_HEADER)?;
    for sub in ["takes", "vo"] {
        std::fs::create_dir_all(dir.join(sub))
            .map_err(|e| vec![format!("can't create {}: {e}", dir.join(sub).display())])?;
    }
    w(dir.join(".gitignore"), "/build/\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_makes_a_project_that_opens() {
        let dir = std::env::temp_dir().join(format!("pixelreel-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        init(&dir, "1280x720", 24, "summer", Some("art/farm.pixellab")).unwrap();
        let p = Project::open(&dir).unwrap();
        assert_eq!(p.config.fps, 24);
        let sb = p.storyboard().unwrap();
        assert_eq!(sb.beats.len(), 3);
        assert!(p.cuts().unwrap().lines.is_empty());
        assert!(
            init(&dir, "1280x720", 24, "summer", None).is_err(),
            "refuses a non-empty folder"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
