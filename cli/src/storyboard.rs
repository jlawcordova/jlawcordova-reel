//! `storyboard.md`: the script and its beats (spec §6.2).

use crate::time::parse_time;
use cozy_score::types::VideoType;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Storyboard {
    pub title: Option<String>,
    pub beats: Vec<Beat>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Beat {
    pub id: String,
    #[serde(serialize_with = "ser_type")]
    pub kind: VideoType,
    pub picture: Picture,
    pub chapter: Option<String>,
    /// `length:` in seconds.
    pub length: Option<f64>,
    pub under: Option<Picture>,
    pub fit: SceneFit,
    /// `from:` on the scene's clock, seconds.
    pub from: f64,
    pub entrance: bool,
    pub cues: Vec<CueAction>,
    pub lines: Vec<Line>,
    /// 1-based line of the heading in storyboard.md.
    pub at: usize,
}

fn ser_type<S: serde::Serializer>(t: &VideoType, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(t.id())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "kebab-case")]
pub enum Picture {
    Face,
    Card,
    Scene(String),
    Asset(String),
}

impl Picture {
    pub fn parse(s: &str) -> Option<Picture> {
        let s = s.trim();
        match s {
            "face" => return Some(Picture::Face),
            "card" => return Some(Picture::Card),
            _ => {}
        }
        let (kind, name) = s.split_once(':')?;
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return None;
        }
        match kind.trim() {
            "scene" => Some(Picture::Scene(name.to_string())),
            "asset" => Some(Picture::Asset(name.to_string())),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Picture::Face => "face".into(),
            Picture::Card => "card".into(),
            Picture::Scene(n) => format!("scene:{n}"),
            Picture::Asset(n) => format!("asset:{n}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SceneFit {
    Loop,
    Hold,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CueAction {
    pub cue: String,
    pub item: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum Voice {
    You,
    Vo(String),
}

impl Voice {
    pub fn label(&self) -> &str {
        match self {
            Voice::You => "you",
            Voice::Vo(v) => v,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    /// `<beat>.<n>`, from 1.
    pub id: String,
    pub voice: Voice,
    /// The text with cue markers taken out.
    pub text: String,
    /// The text's words, split on whitespace.
    pub words: Vec<String>,
    /// Each cue marked in the line, with the index of the word it follows.
    pub cues: Vec<(String, usize)>,
    pub at: usize,
}

const DIRECTIVES: &str = "chapter, length, under, fit, from, entrance, cue <name>";

fn valid_types() -> String {
    VideoType::ALL
        .iter()
        .map(|t| t.id())
        .collect::<Vec<_>>()
        .join(", ")
}

fn valid_beat_id(id: &str) -> bool {
    let mut cs = id.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Blanks out HTML comments, keeping line breaks so line numbers stay.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        match after.find("-->") {
            Some(j) => {
                out.extend(after[..j + 3].chars().filter(|&c| c == '\n'));
                rest = &after[j + 3..];
            }
            None => {
                out.extend(after.chars().filter(|&c| c == '\n'));
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Splits `{#name}` markers out of a line. Returns the clean text and each
/// cue with the index of the word it follows.
fn split_cues(raw: &str, at: usize, errors: &mut Vec<String>) -> (String, Vec<(String, usize)>) {
    let mut text = String::new();
    let mut cues = Vec::new();
    let mut rest = raw;
    while let Some(i) = rest.find("{#") {
        text.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            errors.push(format!(
                "storyboard.md:{at}: a cue marker \"{{#\" is never closed"
            ));
            text.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let name = &after[..j];
        let words_before = text.split_whitespace().count();
        let glued = text.chars().last().is_some_and(|c| !c.is_whitespace());
        if !valid_name(name) {
            errors.push(format!(
                "storyboard.md:{at}: cue name \"{name}\" may only use letters, digits, - and _"
            ));
        } else if words_before == 0 || !glued {
            errors.push(format!(
                "storyboard.md:{at}: cue {{#{name}}} must follow a word with no space between"
            ));
        } else {
            cues.push((name.to_string(), words_before - 1));
        }
        rest = &after[j + 1..];
    }
    text.push_str(rest);
    (text.split_whitespace().collect::<Vec<_>>().join(" "), cues)
}

/// Parses and validates a storyboard. `voices` are the voice ids in
/// pixelreel.yaml. Every problem is reported in one go.
pub fn parse(source: &str, voices: &[String]) -> Result<Storyboard, Vec<String>> {
    let mut errors = Vec::new();
    let text = strip_comments(source);
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut title = None;

    // Front matter.
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    if i < lines.len() && lines[i].trim() == "---" {
        let start = i + 1;
        let mut end = None;
        for (k, l) in lines.iter().enumerate().skip(start) {
            if l.trim() == "---" {
                end = Some(k);
                break;
            }
        }
        match end {
            Some(end) => {
                let fm = lines[start..end].join("\n");
                match serde_yaml::from_str::<serde_yaml::Value>(&fm) {
                    Ok(v) => {
                        title = v
                            .get("title")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string());
                    }
                    Err(e) => errors.push(format!("storyboard.md: front matter: {e}")),
                }
                i = end + 1;
            }
            None => {
                errors.push("storyboard.md: front matter starts with --- but never ends".into());
                i = lines.len();
            }
        }
    }

    let mut beats: Vec<Beat> = Vec::new();
    let mut in_directives = false;
    let mut cue_actions_at: Vec<(String, usize)> = Vec::new();
    while i < lines.len() {
        let at = i + 1;
        let line = lines[i].trim();
        i += 1;
        if let Some(head) = line.strip_prefix("## ") {
            in_directives = true;
            let parts: Vec<&str> = head.split('|').map(str::trim).collect();
            if parts.len() != 3 {
                errors.push(format!(
                    "storyboard.md:{at}: a beat heading is \"## <id> | <type> | <picture>\", got \"{line}\""
                ));
                beats.push(placeholder_beat(at));
                continue;
            }
            let id = parts[0].to_string();
            if !valid_beat_id(&id) {
                errors.push(format!(
                    "storyboard.md:{at}: beat id \"{id}\" may only use a-z, 0-9 and -, starting with a letter or digit"
                ));
            }
            let kind = VideoType::from_id(parts[1]).unwrap_or_else(|| {
                errors.push(format!(
                    "storyboard.md:{at}: unknown video type \"{}\" (valid: {})",
                    parts[1],
                    valid_types()
                ));
                VideoType::Explain
            });
            let picture = Picture::parse(parts[2]).unwrap_or_else(|| {
                errors.push(format!(
                    "storyboard.md:{at}: unknown picture \"{}\" (valid: face, card, scene:<name>, asset:<id>)",
                    parts[2]
                ));
                Picture::Card
            });
            let mut b = placeholder_beat(at);
            b.id = id;
            b.kind = kind;
            b.picture = picture;
            beats.push(b);
            continue;
        }
        if line.is_empty() || line == "…" || line == "..." {
            continue;
        }
        let Some(beat) = beats.last_mut() else {
            // Prose before the first beat is a note.
            continue;
        };
        if in_directives {
            if let Some((key, value)) = directive(line) {
                apply_directive(
                    beat,
                    &key,
                    value.trim(),
                    at,
                    &mut errors,
                    &mut cue_actions_at,
                );
                continue;
            }
        }
        in_directives = false;
        let (voice, body) = match vo_line(line) {
            Some((v, body)) => {
                if !voices.iter().any(|x| x == &v) {
                    let known = if voices.is_empty() {
                        "none are set in pixelreel.yaml".to_string()
                    } else {
                        format!("valid: {}", voices.join(", "))
                    };
                    errors.push(format!(
                        "storyboard.md:{at}: unknown voice \"{v}\" ({known})"
                    ));
                }
                (Voice::Vo(v), body)
            }
            None => (Voice::You, line),
        };
        let (text, cues) = split_cues(body, at, &mut errors);
        if text.is_empty() {
            errors.push(format!("storyboard.md:{at}: the line has no words"));
            continue;
        }
        let words = text.split_whitespace().map(str::to_string).collect();
        let n = beat.lines.len() + 1;
        beat.lines.push(Line {
            id: format!("{}.{n}", beat.id),
            voice,
            text,
            words,
            cues,
            at,
        });
    }

    // Whole-storyboard checks.
    if beats.is_empty() {
        errors.push("storyboard.md: there are no beats; a beat starts with \"## <id> | <type> | <picture>\"".into());
    }
    let mut ids = BTreeSet::new();
    let mut cue_names: BTreeSet<String> = BTreeSet::new();
    for b in &beats {
        if !b.id.is_empty() && !ids.insert(b.id.clone()) {
            errors.push(format!(
                "storyboard.md:{}: beat id \"{}\" is used twice",
                b.at, b.id
            ));
        }
        for l in &b.lines {
            for (c, _) in &l.cues {
                if !cue_names.insert(c.clone()) {
                    errors.push(format!(
                        "storyboard.md:{}: cue {{#{c}}} is marked twice",
                        l.at
                    ));
                }
            }
        }
    }
    for b in &beats {
        if b.lines.is_empty() && b.length.is_none() && !matches!(b.picture, Picture::Asset(_)) {
            errors.push(format!(
                "storyboard.md:{}: beat \"{}\" has no lines, so it needs a length: (or an asset picture)",
                b.at, b.id
            ));
        }
        let is_scene = matches!(b.picture, Picture::Scene(_));
        if b.under.is_some() && !matches!(b.picture, Picture::Asset(_)) {
            errors.push(format!(
                "storyboard.md:{}: under: is only for an asset picture (beat \"{}\")",
                b.at, b.id
            ));
        }
        if !is_scene
            && (b.fit != SceneFit::Loop || b.from != 0.0 || b.entrance || !b.cues.is_empty())
        {
            errors.push(format!(
                "storyboard.md:{}: fit, from, entrance and cue are only for a scene picture (beat \"{}\")",
                b.at, b.id
            ));
        }
    }
    for b in &beats {
        if let Picture::Scene(name) = &b.picture {
            if name.starts_with("sequences/") && (b.entrance || !b.cues.is_empty()) {
                errors.push(format!(
                    "storyboard.md:{}: {name} is a sequence, which plays whole; entrance and cue are for a scene (beat \"{}\")",
                    b.at, b.id
                ));
            }
        }
    }
    for (c, at) in &cue_actions_at {
        if !cue_names.contains(c) {
            errors.push(format!(
                "storyboard.md:{at}: cue \"{c}\" isn't marked in any line (mark a word with {{#{c}}})"
            ));
        }
    }

    if errors.is_empty() {
        Ok(Storyboard { title, beats })
    } else {
        Err(errors)
    }
}

fn placeholder_beat(at: usize) -> Beat {
    Beat {
        id: String::new(),
        kind: VideoType::Explain,
        picture: Picture::Card,
        chapter: None,
        length: None,
        under: None,
        fit: SceneFit::Loop,
        from: 0.0,
        entrance: false,
        cues: Vec::new(),
        lines: Vec::new(),
        at,
    }
}

/// `key: value` where key is one lowercase word, or `cue <name>`.
fn directive(line: &str) -> Option<(String, &str)> {
    let (key, value) = line.split_once(':')?;
    if let Some(name) = key.strip_prefix("cue ") {
        return valid_name(name.trim()).then(|| (format!("cue {}", name.trim()), value));
    }
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
        .then(|| (key.to_string(), value))
}

fn vo_line(line: &str) -> Option<(String, &str)> {
    let rest = line.strip_prefix("VO(")?;
    let (voice, body) = rest.split_once("):")?;
    Some((voice.trim().to_string(), body.trim()))
}

fn apply_directive(
    beat: &mut Beat,
    key: &str,
    value: &str,
    at: usize,
    errors: &mut Vec<String>,
    cue_actions_at: &mut Vec<(String, usize)>,
) {
    let ctx = format!("storyboard.md:{at}");
    match key {
        "chapter" => {
            if value.is_empty() {
                errors.push(format!("{ctx}: chapter: needs a title"));
            } else {
                beat.chapter = Some(value.to_string());
            }
        }
        "length" => match parse_time(value) {
            Some(v) if v > 0.0 => beat.length = Some(v),
            _ => errors.push(format!(
                "{ctx}: length \"{value}\" must be a time above 0, like 4s, 23.5s or 0:23"
            )),
        },
        "under" => match Picture::parse(value) {
            Some(p @ (Picture::Face | Picture::Scene(_))) => beat.under = Some(p),
            _ => errors.push(format!(
                "{ctx}: under \"{value}\" must be face or scene:<name>"
            )),
        },
        "fit" => match value {
            "loop" => beat.fit = SceneFit::Loop,
            "hold" => beat.fit = SceneFit::Hold,
            _ => errors.push(format!("{ctx}: fit \"{value}\" must be loop or hold")),
        },
        "from" => match parse_time(value) {
            Some(v) => beat.from = v,
            None => errors.push(format!(
                "{ctx}: from \"{value}\" must be a time, like 2s or 1500ms"
            )),
        },
        "entrance" => match value {
            "true" | "yes" => beat.entrance = true,
            "false" | "no" => beat.entrance = false,
            _ => errors.push(format!("{ctx}: entrance \"{value}\" must be true or false")),
        },
        k if k.starts_with("cue ") => {
            let cue = k["cue ".len()..].to_string();
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() != 2 {
                errors.push(format!(
                    "{ctx}: \"cue {cue}:\" takes an item id and a state, like \"cue {cue}: sprinkler-1 spray\""
                ));
                return;
            }
            if beat.cues.iter().any(|c| c.cue == cue) {
                errors.push(format!("{ctx}: cue \"{cue}\" is set twice in this beat"));
            }
            cue_actions_at.push((cue.clone(), at));
            beat.cues.push(CueAction {
                cue,
                item: parts[0].to_string(),
                state: parts[1].to_string(),
            });
        }
        other => errors.push(format!(
            "{ctx}: unknown directive \"{other}:\" (valid: {DIRECTIVES})"
        )),
    }
}

/// Every word that a cue is marked on, by cue name: (line id, word index).
pub fn cue_marks(sb: &Storyboard) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    for b in &sb.beats {
        for l in &b.lines {
            for (c, w) in &l.cues {
                out.push((c.clone(), l.id.clone(), *w));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voices() -> Vec<String> {
        vec!["sprout".into()]
    }

    #[test]
    fn parses_the_sample() {
        let src = "---\ntitle: Farm Devlog\n---\n\n# notes before beats are ignored\n\n## hook | cold-open-hook | face\nchapter: Can a farm run itself?\nThirty days ago this field was empty.\nToday it plants, waters and harvests itself.\n\n## hook-tease | cold-open-hook | scene:farm-night\nfit: hold\nfrom: 1500ms\nentrance: true\ncue lights-on: lamp-1 on\nAnd I haven't touched it in a week{#lights-on}.\n<!-- a note\nover lines -->\n## title | title-sting | asset:logo-intro\nlength: 4s\n\n## demo | visual-demo | scene:sequences/tour\nVO(sprout): Day one. Soil quality: questionable{#shrug}.\n…\n";
        let sb = parse(src, &voices()).unwrap();
        assert_eq!(sb.title.as_deref(), Some("Farm Devlog"));
        assert_eq!(sb.beats.len(), 4);
        let hook = &sb.beats[0];
        assert_eq!(hook.chapter.as_deref(), Some("Can a farm run itself?"));
        assert_eq!(hook.lines.len(), 2);
        assert_eq!(hook.lines[1].id, "hook.2");
        let tease = &sb.beats[1];
        assert_eq!(tease.picture, Picture::Scene("farm-night".into()));
        assert_eq!(tease.fit, SceneFit::Hold);
        assert_eq!(tease.from, 1.5);
        assert!(tease.entrance);
        assert_eq!(tease.cues[0].item, "lamp-1");
        assert_eq!(tease.lines[0].text, "And I haven't touched it in a week.");
        assert_eq!(tease.lines[0].cues, vec![("lights-on".to_string(), 7)]);
        assert_eq!(sb.beats[2].at, 20, "comments keep line numbers");
        assert_eq!(sb.beats[2].length, Some(4.0));
        let demo = &sb.beats[3];
        assert_eq!(demo.lines[0].voice, Voice::Vo("sprout".into()));
        assert_eq!(demo.lines[0].text, "Day one. Soil quality: questionable.");
        assert_eq!(demo.lines[0].cues, vec![("shrug".to_string(), 4)]);
    }

    #[test]
    fn a_script_line_that_looks_like_a_directive_after_text_is_text() {
        let src = "## a | explain | face\nHello there.\nnote: this is spoken.\n";
        let sb = parse(src, &[]).unwrap();
        assert_eq!(sb.beats[0].lines.len(), 2);
    }

    #[test]
    fn reports_every_error_at_once() {
        let src = "## Bad_Id | explaining | video\nchapter:\nfoo: bar\nlength: soon\n## ok | explain | face\nVO(robot): Hi{#x}.\n## ok | reveal | card\n## s | reveal | face\nfit: hold\ncue y: a b\nunder: face\nSay{#x} it.\n{#z}Oops.\n## q | reveal | scene:sequences/tour\nentrance: true\nlength: 3s\n";
        let e = parse(src, &voices()).unwrap_err();
        let joined = e.join("\n");
        for needle in [
            "beat id \"Bad_Id\"",
            "unknown video type \"explaining\"",
            "unknown picture \"video\"",
            "chapter: needs a title",
            "unknown directive \"foo:\"",
            "length \"soon\"",
            "unknown voice \"robot\"",
            "\"ok\" is used twice",
            "cue {#x} is marked twice",
            "only for a scene picture",
            "only for an asset picture",
            "cue \"y\" isn't marked",
            "must follow a word",
            "beat \"ok\" has no lines",
            "is a sequence, which plays whole",
        ] {
            assert!(joined.contains(needle), "missing {needle:?} in:\n{joined}");
        }
    }

    #[test]
    fn an_empty_storyboard_is_an_error() {
        let e = parse("just notes\n", &[]).unwrap_err();
        assert!(e[0].contains("no beats"));
    }

    #[test]
    fn a_heading_needs_three_parts() {
        let e = parse("## a | explain\nHi.\n", &[]).unwrap_err();
        assert!(e[0].contains("## <id> | <type> | <picture>"), "{e:?}");
    }
}
