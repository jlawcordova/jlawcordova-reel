//! `pixelreel.yaml`: the project's settings (spec §6.1).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Output size, `WxH`.
    pub size: String,
    pub fps: u32,
    pub feel: String,
    pub pixellab: PixelLab,
    pub voices: BTreeMap<String, Voice>,
    pub speech: Speech,
    pub transcribe: Transcribe,
    pub cut: Cut,
    pub mix: Mix,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            size: "1920x1080".into(),
            fps: 30,
            feel: "spring".into(),
            pixellab: PixelLab::default(),
            voices: BTreeMap::new(),
            speech: Speech::default(),
            transcribe: Transcribe::default(),
            cut: Cut::default(),
            mix: Mix::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct PixelLab {
    pub bin: String,
    /// The `.pixellab` document, relative to the project.
    pub document: Option<String>,
    pub background: String,
    pub theme: String,
}

impl Default for PixelLab {
    fn default() -> Self {
        PixelLab {
            bin: "pixellab".into(),
            document: None,
            background: "card".into(),
            theme: "light".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Voice {
    pub voice_id: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_stability")]
    pub stability: f64,
    #[serde(default = "default_similarity")]
    pub similarity: f64,
}

fn default_model() -> String {
    "eleven_multilingual_v2".into()
}
fn default_stability() -> f64 {
    0.5
}
fn default_similarity() -> f64 {
    0.75
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Speech {
    pub wpm: f64,
    pub pause: f64,
}

impl Default for Speech {
    fn default() -> Self {
        Speech {
            wpm: 155.0,
            pause: 0.25,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Transcribe {
    pub bin: String,
    pub model: Option<String>,
}

impl Default for Transcribe {
    fn default() -> Self {
        Transcribe {
            bin: "whisper-cli".into(),
            model: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Cut {
    pub pre_roll: f64,
    pub keep_breath: f64,
    pub max_gap: f64,
}

impl Default for Cut {
    fn default() -> Self {
        Cut {
            pre_roll: 0.08,
            keep_breath: 0.12,
            max_gap: 0.30,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Mix {
    pub duck_db: f64,
    pub duck_attack_ms: f64,
    pub duck_release_ms: f64,
    pub target_lufs: f64,
    pub true_peak_dbtp: f64,
}

impl Default for Mix {
    fn default() -> Self {
        Mix {
            duck_db: -9.0,
            duck_attack_ms: 60.0,
            duck_release_ms: 350.0,
            target_lufs: -14.0,
            true_peak_dbtp: -1.0,
        }
    }
}

/// Parses `WxH`.
pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let (w, h): (u32, u32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w >= 16 && h >= 16 && w % 2 == 0 && h % 2 == 0).then_some((w, h))
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, Vec<String>> {
        let c: Config = if text.trim().is_empty() {
            Config::default()
        } else {
            serde_yaml::from_str(text).map_err(|e| vec![format!("pixelreel.yaml: {e}")])?
        };
        let errors = c.validate();
        if errors.is_empty() {
            Ok(c)
        } else {
            Err(errors)
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut e = Vec::new();
        if parse_size(&self.size).is_none() {
            e.push(format!(
                "pixelreel.yaml: size \"{}\" must be WxH with even numbers of at least 16, like 1920x1080",
                self.size
            ));
        }
        if ![12, 24, 30, 60].contains(&self.fps) {
            e.push(format!(
                "pixelreel.yaml: fps {} must be 12, 24, 30 or 60 (the rates Pixel Lab exports)",
                self.fps
            ));
        }
        if !matches!(self.feel.as_str(), "spring" | "summer") {
            e.push(format!(
                "pixelreel.yaml: feel \"{}\" must be spring or summer",
                self.feel
            ));
        }
        if !matches!(self.pixellab.theme.as_str(), "light" | "dark") {
            e.push(format!(
                "pixelreel.yaml: pixellab.theme \"{}\" must be light or dark",
                self.pixellab.theme
            ));
        }
        if self.speech.wpm <= 0.0 {
            e.push("pixelreel.yaml: speech.wpm must be above 0".into());
        }
        if self.speech.pause < 0.0 {
            e.push("pixelreel.yaml: speech.pause can't be negative".into());
        }
        for (k, v) in [
            ("cut.pre_roll", self.cut.pre_roll),
            ("cut.keep_breath", self.cut.keep_breath),
            ("cut.max_gap", self.cut.max_gap),
        ] {
            if !(0.0..=2.0).contains(&v) {
                e.push(format!("pixelreel.yaml: {k} must be 0 to 2 seconds"));
            }
        }
        if self.mix.duck_db > 0.0 {
            e.push("pixelreel.yaml: mix.duck_db must be 0 or below".into());
        }
        if self.mix.duck_attack_ms <= 0.0 || self.mix.duck_release_ms <= 0.0 {
            e.push("pixelreel.yaml: mix.duck_attack_ms and duck_release_ms must be above 0".into());
        }
        if self.mix.true_peak_dbtp > 0.0 {
            e.push("pixelreel.yaml: mix.true_peak_dbtp must be 0 or below".into());
        }
        for (id, v) in &self.voices {
            if v.voice_id.trim().is_empty() {
                e.push(format!("pixelreel.yaml: voices.{id}.voice_id is empty"));
            }
        }
        e
    }

    pub fn size(&self) -> (u32, u32) {
        parse_size(&self.size).unwrap_or((1920, 1080))
    }

    pub fn feel(&self) -> cozy_score::types::Feel {
        match self.feel.as_str() {
            "summer" => cozy_score::types::Feel::Summer,
            _ => cozy_score::types::Feel::Spring,
        }
    }

    /// The file `init` writes: every default filled in, with comments.
    pub fn template(size: &str, fps: u32, feel: &str, document: Option<&str>) -> String {
        let doc = match document {
            Some(d) => format!("  document: {}\n", yaml_str(d)),
            None => "  # document: art/farm.pixellab   # needed for scene: beats\n".into(),
        };
        format!(
            "\
# pixelreel project settings (spec §6.1)
size: {size}          # output size; Pixel Lab fits scenes at the largest whole scale
fps: {fps}                  # 12, 24, 30 or 60
feel: {feel}             # cozy-score feel: spring | summer
pixellab:
  bin: pixellab
{doc}  background: card       # card, dark, black or #RRGGBB
  theme: light
voices: {{}}
  # sprout:
  #   voice_id: \"<elevenlabs-voice-id>\"
  #   model: eleven_multilingual_v2
  #   stability: 0.5
  #   similarity: 0.75
speech:
  wpm: 155               # estimate for unrecorded lines
  pause: 0.25            # estimated pause between lines, seconds
transcribe:
  bin: whisper-cli
  # model: ~/models/ggml-base.en.bin
cut:
  pre_roll: 0.08         # kept before a line's first word
  keep_breath: 0.12      # kept after a line's last word
  max_gap: 0.3           # longest natural pause kept between back-to-back lines
mix:
  duck_db: -9
  duck_attack_ms: 60
  duck_release_ms: 350
  target_lufs: -14
  true_peak_dbtp: -1
"
        )
    }
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        let c = Config::parse(&Config::template("1920x1080", 30, "spring", None)).unwrap();
        assert_eq!(c, Config::default());
        let c = Config::parse(&Config::template(
            "1280x720",
            24,
            "summer",
            Some("a b.pixellab"),
        ))
        .unwrap();
        assert_eq!(c.size(), (1280, 720));
        assert_eq!(c.pixellab.document.as_deref(), Some("a b.pixellab"));
    }

    #[test]
    fn reports_every_problem() {
        let e = Config::parse("size: 1920\nfps: 25\nfeel: winter\n").unwrap_err();
        assert_eq!(e.len(), 3, "{e:?}");
        let e = Config::parse("nonsense: 1\n").unwrap_err();
        assert!(e[0].contains("nonsense"), "{e:?}");
    }
}
