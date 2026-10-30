//! The timeline: estimated from the script, or locked from takes and VO
//! audio (spec §6.5, §7.3, §8).

use crate::align::{Read, TWord};
use crate::config::Config;
use crate::files::{AssetFit, Assets, Cuts};
use crate::storyboard::{Beat, Line, Picture, Storyboard, Voice};
use crate::time::{to_frame, to_ms};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub pixelreel: String,
    pub locked: bool,
    /// The hash of everything below, so a re-lock that changes nothing is
    /// recognised.
    pub inputs: String,
    pub fps: u32,
    pub duration: f64,
    pub beats: Vec<TBeat>,
    /// Cue name → time.
    pub cues: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TBeat {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub picture: String,
    /// Picture span.
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<String>,
    /// Voice span: lines are laid end to end from here.
    pub voice_start: f64,
    pub voice_end: f64,
    pub lines: Vec<TLine>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TLine {
    pub id: String,
    /// `you` or a voice id.
    pub voice: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    pub words: Vec<TWord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// The take's name, for a you-line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<String>,
    /// The media file, as stored (relative to the project, or absolute).
    pub file: String,
    #[serde(rename = "in")]
    pub in_: f64,
    pub out: f64,
}

/// Audio for a line that already exists: a picked VO take, or draft TTS.
#[derive(Clone, Debug)]
pub struct LineAudio {
    pub file: String,
    pub duration: f64,
    pub words: Vec<TWord>,
}

/// One line's audio, before it is placed: its length, source, and its
/// words relative to the line's start.
struct Seg {
    dur: f64,
    source: Option<Source>,
    words: Vec<(f64, f64)>,
}

impl Timeline {
    pub fn beat(&self, id: &str) -> Option<&TBeat> {
        self.beats.iter().find(|b| b.id == id)
    }
    pub fn lines(&self) -> impl Iterator<Item = &TLine> {
        self.beats.iter().flat_map(|b| b.lines.iter())
    }
}

/// Trims VO audio to its first and last word with the cut padding.
fn trim_audio(a: &LineAudio, line: &Line, cut: &crate::config::Cut) -> Seg {
    let first = a.words.first().map_or(0.0, |w| w.start);
    let last = a.words.last().map_or(a.duration, |w| w.end);
    let in_ = (first - cut.pre_roll).max(0.0);
    let out = (last + cut.keep_breath).min(a.duration.max(last));
    Seg {
        dur: (out - in_).max(0.01),
        source: Some(Source {
            take: None,
            file: a.file.clone(),
            in_,
            out,
        }),
        words: script_word_times(line, &a.words, in_),
    }
}

/// Times for each script word from a run of spoken words, relative to
/// `origin`. Spoken words map onto script words by index, as both are the
/// line's text split on whitespace; extra or missing words stretch evenly.
fn script_word_times(line: &Line, spoken: &[TWord], origin: f64) -> Vec<(f64, f64)> {
    let n = line.words.len();
    if spoken.is_empty() {
        return vec![(0.0, 0.0); n];
    }
    if spoken.len() == n {
        return spoken
            .iter()
            .map(|w| (w.start - origin, w.end - origin))
            .collect();
    }
    let (s0, e0) = (spoken[0].start, spoken[spoken.len() - 1].end);
    let slot = (e0 - s0) / n.max(1) as f64;
    (0..n)
        .map(|i| {
            (
                s0 + i as f64 * slot - origin,
                s0 + (i + 1) as f64 * slot - origin,
            )
        })
        .collect()
}

fn estimate_seg(line: &Line, wpm: f64) -> Seg {
    let n = line.words.len().max(1);
    let dur = n as f64 * 60.0 / wpm;
    let slot = dur / n as f64;
    Seg {
        dur,
        source: None,
        words: (0..line.words.len())
            .map(|i| (i as f64 * slot, (i + 1) as f64 * slot))
            .collect(),
    }
}

/// The asset-driven minimum length of a beat, if any.
fn asset_min(
    beat: &Beat,
    assets: &Assets,
    warnings: &mut Vec<String>,
    locked: bool,
    errors: &mut Vec<String>,
) -> Option<f64> {
    let Picture::Asset(id) = &beat.picture else {
        return None;
    };
    match assets.get(id) {
        Some(a) => {
            if a.fit == AssetFit::HoldLast || (beat.lines.is_empty() && beat.length.is_none()) {
                Some(a.duration)
            } else {
                None
            }
        }
        None => {
            if beat.lines.is_empty() && beat.length.is_none() {
                let msg = format!(
                    "beat \"{}\" shows asset \"{id}\", which isn't registered (pixelreel asset add <file> --id {id})",
                    beat.id
                );
                if locked {
                    errors.push(msg);
                } else {
                    warnings.push(format!("{msg}; assuming 4 s"));
                }
                Some(4.0)
            } else {
                None
            }
        }
    }
}

/// Lays the lines out end to end and works out the beat boundaries.
#[allow(clippy::too_many_arguments)]
fn layout(
    sb: &Storyboard,
    cfg: &Config,
    cuts: &Cuts,
    assets: &Assets,
    segs: Vec<Vec<Seg>>,
    pause: f64,
    locked: bool,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Timeline {
    let fps = cfg.fps;
    let mut cursor = 0.0f64;
    let mut beats = Vec::new();
    let mut voice_starts = Vec::new();
    let mut cues = BTreeMap::new();
    for (bi, (beat, bsegs)) in sb.beats.iter().zip(segs).enumerate() {
        let voice_start = cursor;
        let mut lines = Vec::new();
        for (li, (line, seg)) in beat.lines.iter().zip(bsegs).enumerate() {
            if li > 0 {
                cursor += pause;
            }
            let start = cursor;
            let end = cursor + seg.dur;
            let words: Vec<TWord> = line
                .words
                .iter()
                .zip(&seg.words)
                .map(|(w, &(s, e))| TWord {
                    w: w.clone(),
                    start: to_ms(start + s.clamp(0.0, seg.dur)),
                    end: to_ms(start + e.clamp(0.0, seg.dur)),
                })
                .collect();
            for (c, wi) in &line.cues {
                if let Some(w) = words.get(*wi) {
                    cues.insert(c.clone(), w.start);
                }
            }
            lines.push(TLine {
                id: line.id.clone(),
                voice: line.voice.label().to_string(),
                text: line.text.clone(),
                start: to_ms(start),
                end: to_ms(end),
                source: seg.source,
                words,
            });
            cursor = end;
        }
        let natural = cursor - voice_start;
        let mut min = beat.length.unwrap_or(0.0);
        if let Some(a) = asset_min(beat, assets, warnings, locked, errors) {
            min = min.max(a);
        }
        let bc = cuts.beats.get(&beat.id).cloned().unwrap_or_default();
        let mut len = natural.max(min) + bc.length.unwrap_or(0.0);
        if len < natural - 1e-6 {
            let msg = format!(
                "beat \"{}\": a length change of {:+.3} s would cut into its voice ({:.3} s); the shortest it can be is its voice",
                beat.id,
                bc.length.unwrap_or(0.0),
                natural
            );
            if locked {
                errors.push(msg);
            } else {
                warnings.push(msg);
            }
            len = natural;
        }
        if len <= 0.0 {
            errors.push(format!("beat \"{}\" has no length", beat.id));
            len = 1.0 / f64::from(fps);
        }
        cursor = voice_start + len;
        let lead = bc.lead.unwrap_or(0.0);
        if bi == 0 && lead != 0.0 {
            errors.push(format!(
                "beat \"{}\" is the first beat, so it can't have a lead (J-cut)",
                beat.id
            ));
        }
        voice_starts.push(voice_start + if bi == 0 { 0.0 } else { lead });
        beats.push(TBeat {
            id: beat.id.clone(),
            kind: beat.kind.id().to_string(),
            picture: beat.picture.label(),
            start: 0.0,
            end: 0.0,
            chapter: beat.chapter.clone(),
            voice_start: to_ms(voice_start),
            voice_end: to_ms(voice_start + natural),
            lines,
        });
    }
    // Picture boundaries on the frame grid.
    let frame = 1.0 / f64::from(fps);
    let total = (cursor * f64::from(fps) - 1e-6).ceil().max(1.0) / f64::from(fps);
    let n = beats.len();
    for i in 0..n {
        let start = if i == 0 {
            0.0
        } else {
            to_frame(voice_starts[i], fps)
        };
        let end = if i + 1 == n {
            total
        } else {
            to_frame(voice_starts[i + 1], fps)
        };
        if end < start + frame - 1e-9 {
            errors.push(format!(
                "beat \"{}\" has no picture left: the next beat's lead (J-cut) is longer than it",
                beats[i].id
            ));
        }
        beats[i].start = to_ms(start);
        beats[i].end = to_ms(end);
    }
    let mut t = Timeline {
        pixelreel: VERSION.to_string(),
        locked,
        inputs: String::new(),
        fps,
        duration: to_ms(total),
        beats,
        cues,
    };
    t.inputs = body_hash(&t);
    t
}

/// The hash of a timeline's content, `inputs` left out.
pub fn body_hash(t: &Timeline) -> String {
    let mut v = serde_json::to_value(t).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.remove("inputs");
        o.remove("pixelreel");
    }
    crate::hash::of_json(&v)
}

/// The estimated timeline (spec §7.3). `audio` gives existing audio for a
/// line (a picked VO take, or draft TTS); other lines are estimated from
/// `wpm`.
pub fn estimate(
    sb: &Storyboard,
    cfg: &Config,
    cuts: &Cuts,
    assets: &Assets,
    wpm: f64,
    audio: &dyn Fn(&Line) -> Option<LineAudio>,
) -> (Timeline, Vec<String>) {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let segs = sb
        .beats
        .iter()
        .map(|b| {
            b.lines
                .iter()
                .map(|l| match audio(l) {
                    Some(a) => trim_audio(&a, l, &cfg.cut),
                    None => estimate_seg(l, wpm),
                })
                .collect()
        })
        .collect();
    let t = layout(
        sb,
        cfg,
        cuts,
        assets,
        segs,
        cfg.speech.pause,
        false,
        &mut warnings,
        &mut errors,
    );
    warnings.extend(errors);
    (t, warnings)
}

/// What `lock` needs besides the storyboard and settings.
pub struct LockInputs<'a> {
    /// Every read of every line, in take order and then time.
    pub reads: &'a BTreeMap<String, Vec<Read>>,
    /// Take name → (stored file path, duration).
    pub takes: &'a BTreeMap<String, (String, f64)>,
    /// A VO line's picked audio.
    pub vo: &'a dyn Fn(&Line) -> Option<LineAudio>,
}

/// The cut settings in force: pixelreel.yaml, overridden by `cut tighten`.
pub fn cut_settings(cfg: &Config, cuts: &Cuts) -> crate::config::Cut {
    let mut c = cfg.cut;
    if let Some(t) = cuts.tighten {
        if let Some(v) = t.max_gap {
            c.max_gap = v;
        }
        if let Some(v) = t.keep_breath {
            c.keep_breath = v;
        }
    }
    c
}

/// Chooses a you-line's read: the last complete one, in the take that
/// `cuts.yaml` names if it names one.
pub fn choose_read<'a>(
    line: &str,
    reads: &'a BTreeMap<String, Vec<Read>>,
    take: Option<&str>,
) -> Option<&'a Read> {
    reads
        .get(line)?
        .iter()
        .rfind(|r| r.complete && take.is_none_or(|t| r.take == t))
}

/// The locked timeline (spec §7.8 `lock`, §8). Fails, naming every
/// problem, when a line has no read or audio.
pub fn lock(
    sb: &Storyboard,
    cfg: &Config,
    cuts: &Cuts,
    assets: &Assets,
    inputs: &LockInputs,
) -> Result<(Timeline, Vec<String>), Vec<String>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let cut = cut_settings(cfg, cuts);

    // Raw in/out per you-line from its read, before joins.
    struct You<'a> {
        read: &'a Read,
        in_: f64,
        out: f64,
    }
    let mut you: BTreeMap<String, You> = BTreeMap::new();
    for b in &sb.beats {
        let you_lines = b.lines.iter().filter(|l| l.voice == Voice::You).count();
        if b.picture == Picture::Face && you_lines == 0 {
            errors.push(format!(
                "beat \"{}\" shows face but has no lines you read; give it a line or another picture",
                b.id
            ));
        }
        if b.under == Some(Picture::Face) && you_lines == 0 {
            errors.push(format!(
                "beat \"{}\" has under: face but no lines you read",
                b.id
            ));
        }
        for l in b.lines.iter().filter(|l| l.voice == Voice::You) {
            let want = cuts.lines.get(&l.id).and_then(|c| c.take.as_deref());
            match choose_read(&l.id, inputs.reads, want) {
                Some(r) => {
                    you.insert(
                        l.id.clone(),
                        You {
                            read: r,
                            in_: r.start - cut.pre_roll,
                            out: r.end + cut.keep_breath,
                        },
                    );
                }
                None => errors.push(match want {
                    Some(t) => {
                        format!("{} has no complete read in take {t} (\"{}\")", l.id, l.text)
                    }
                    None => format!("{} has no complete read in any take (\"{}\")", l.id, l.text),
                }),
            }
        }
    }

    // Joins between consecutive lines read back to back in the same take.
    let order: Vec<&Line> = sb.beats.iter().flat_map(|b| b.lines.iter()).collect();
    for pair in order.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (Some(ra), Some(rb)) = (
            you.get(&a.id).map(|y| y.read),
            you.get(&b.id).map(|y| y.read),
        ) else {
            continue;
        };
        let back_to_back = ra.take == rb.take
            && rb.first_word > ra.last_word
            && rb.first_word - ra.last_word <= 3
            && rb.start >= ra.end
            && rb.start - ra.end <= 2.0;
        if !back_to_back {
            continue;
        }
        let p = rb.start - ra.end;
        let (a_out, b_in) = if p <= cut.max_gap {
            let mid = ra.end + p / 2.0;
            (mid, mid)
        } else {
            (
                ra.end + cut.keep_breath.min(p / 2.0),
                rb.start - cut.pre_roll.min(p / 2.0),
            )
        };
        you.get_mut(&a.id).unwrap().out = a_out;
        you.get_mut(&b.id).unwrap().in_ = b_in;
    }

    let mut segs: Vec<Vec<Seg>> = Vec::new();
    for b in &sb.beats {
        let mut bs = Vec::new();
        for l in &b.lines {
            let lc = cuts.lines.get(&l.id).cloned().unwrap_or_default();
            let seg = match &l.voice {
                Voice::You => match you.get(&l.id) {
                    Some(y) => {
                        let (file, tdur) = inputs
                            .takes
                            .get(&y.read.take)
                            .cloned()
                            .unwrap_or((String::new(), f64::MAX));
                        let in_ = (y.in_ + lc.in_.unwrap_or(0.0)).max(0.0);
                        let out = (y.out + lc.out.unwrap_or(0.0)).min(tdur);
                        if out < in_ + 0.05 {
                            errors.push(format!(
                                "{}: its in and out points leave less than 0.05 s (in {in_:.3}, out {out:.3})",
                                l.id
                            ));
                        }
                        Seg {
                            dur: (out - in_).max(0.05),
                            source: Some(Source {
                                take: Some(y.read.take.clone()),
                                file,
                                in_: to_ms(in_),
                                out: to_ms(out),
                            }),
                            words: y
                                .read
                                .words
                                .iter()
                                .map(|&(s, e)| (s - in_, e - in_))
                                .collect(),
                        }
                    }
                    None => estimate_seg(l, cfg.speech.wpm),
                },
                Voice::Vo(v) => match (inputs.vo)(l) {
                    Some(a) => {
                        let mut s = trim_audio(&a, l, &cut);
                        if let Some(src) = s.source.as_mut() {
                            src.in_ = (src.in_ + lc.in_.unwrap_or(0.0)).max(0.0);
                            src.out = (src.out + lc.out.unwrap_or(0.0)).min(a.duration);
                            let shift = lc.in_.unwrap_or(0.0);
                            s.words.iter_mut().for_each(|w| {
                                w.0 -= shift;
                                w.1 -= shift;
                            });
                            s.dur = (src.out - src.in_).max(0.05);
                            src.in_ = to_ms(src.in_);
                            src.out = to_ms(src.out);
                        }
                        s
                    }
                    None => {
                        errors.push(format!(
                            "{} (voice {v}) has no audio; run pixelreel vo gen {}",
                            l.id, b.id
                        ));
                        estimate_seg(l, cfg.speech.wpm)
                    }
                },
            };
            bs.push(seg);
        }
        segs.push(bs);
    }
    let t = layout(
        sb,
        cfg,
        cuts,
        assets,
        segs,
        0.0,
        true,
        &mut warnings,
        &mut errors,
    );
    for (cue, at) in crate::storyboard::cue_marks(sb)
        .iter()
        .map(|(c, _, _)| (c, t.cues.get(c)))
    {
        if at.is_none() {
            warnings.push(format!("cue {cue} has no time"));
        }
    }
    if errors.is_empty() {
        Ok((t, warnings))
    } else {
        Err(errors)
    }
}

/// The measured speaking rate from complete reads, if there are any.
pub fn measured_wpm(reads: &BTreeMap<String, Vec<Read>>) -> Option<f64> {
    let mut rates: Vec<f64> = reads
        .values()
        .filter_map(|rs| rs.iter().rfind(|r| r.complete))
        .filter(|r| r.end - r.start > 0.3)
        .map(|r| r.total as f64 * 60.0 / (r.end - r.start))
        .collect();
    if rates.len() < 2 {
        return None;
    }
    rates.sort_by(f64::total_cmp);
    Some(rates[rates.len() / 2])
}

/// Reads and writes `build/timeline.json`.
pub fn load(path: &std::path::Path) -> Option<Timeline> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(path: &std::path::Path, t: &Timeline) -> Result<(), Vec<String>> {
    let mut text = serde_json::to_string_pretty(t).map_err(|e| vec![e.to_string()])?;
    text.push('\n');
    crate::files::write_atomic(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::{AssetAudio, AssetEntry, BeatCut, LineCut};
    use crate::storyboard::parse;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.voices.insert(
            "sprout".into(),
            crate::config::Voice {
                voice_id: "v".into(),
                model: "m".into(),
                stability: 0.5,
                similarity: 0.75,
            },
        );
        c
    }

    fn read(take: &str, line: &str, n: usize, t0: f64, first_word: usize) -> Read {
        Read {
            take: take.into(),
            line: line.into(),
            score: 2 * n as i32,
            matched: n,
            total: n,
            complete: true,
            first_word,
            last_word: first_word + n - 1,
            start: t0,
            end: t0 + n as f64 * 0.5 - 0.1,
            words: (0..n)
                .map(|i| (t0 + i as f64 * 0.5, t0 + i as f64 * 0.5 + 0.4))
                .collect(),
        }
    }

    const SB: &str = "## hook | cold-open-hook | face\nchapter: Hook\nOne two three four.\nFive six seven eight.\n## demo | visual-demo | scene:farm\nchapter: Demo\nVO(sprout): Hello there robot{#hi}.\n## title | title-sting | asset:logo\n## end | outro-end-screen | card\nchapter: End\nlength: 20s\n";

    fn assets() -> Assets {
        let mut a = Assets::new();
        a.insert(
            "logo".into(),
            AssetEntry {
                path: "logo.mov".into(),
                sha256: "x".into(),
                duration: 4.0,
                size: "1920x1080".into(),
                fit: AssetFit::Trim,
                alpha: false,
                audio: AssetAudio::Keep,
                has_audio: false,
            },
        );
        a
    }

    fn vo(_: &Line) -> Option<LineAudio> {
        Some(LineAudio {
            file: "vo/abc-1.wav".into(),
            duration: 2.0,
            words: vec![
                TWord {
                    w: "Hello".into(),
                    start: 0.3,
                    end: 0.6,
                },
                TWord {
                    w: "there".into(),
                    start: 0.7,
                    end: 1.0,
                },
                TWord {
                    w: "robot.".into(),
                    start: 1.1,
                    end: 1.5,
                },
            ],
        })
    }

    fn inputs_reads() -> BTreeMap<String, Vec<Read>> {
        let mut m = BTreeMap::new();
        // hook.1 read twice; the second read is back to back with hook.2.
        m.insert(
            "hook.1".into(),
            vec![
                read("A001", "hook.1", 4, 1.0, 0),
                read("A001", "hook.1", 4, 5.0, 4),
            ],
        );
        m.insert("hook.2".into(), vec![read("A001", "hook.2", 4, 7.1, 8)]);
        m
    }

    #[test]
    fn locks_with_joins_vo_trim_assets_and_frame_grid() {
        let sb = parse(SB, &["sprout".to_string()]).unwrap();
        let reads = inputs_reads();
        let mut takes = BTreeMap::new();
        takes.insert("A001".to_string(), ("takes/A001.mp4".to_string(), 60.0));
        let inputs = LockInputs {
            reads: &reads,
            takes: &takes,
            vo: &vo,
        };
        let (t, _) = lock(&sb, &cfg(), &Cuts::default(), &assets(), &inputs).unwrap();
        let hook = &t.beats[0];
        let l1 = &hook.lines[0];
        let l2 = &hook.lines[1];
        // The last complete read wins: 5.0..6.9, then 7.1..8.9; the pause is
        // 0.2 s, under max_gap, so the cut sits in its middle with no gap.
        let s1 = l1.source.as_ref().unwrap();
        let s2 = l2.source.as_ref().unwrap();
        assert!((s1.in_ - 4.92).abs() < 1e-9, "{s1:?}");
        assert!((s1.out - 7.0).abs() < 1e-9, "{s1:?}");
        assert!((s2.in_ - 7.0).abs() < 1e-9, "{s2:?}");
        assert!((s2.out - 9.12).abs() < 1e-9, "{s2:?}");
        assert_eq!(l1.start, 0.0);
        assert!((l2.start - 2.08).abs() < 1e-9);
        // VO trimmed to 0.22..1.62.
        let demo = &t.beats[1];
        let vs = demo.lines[0].source.as_ref().unwrap();
        assert!(
            (vs.in_ - 0.22).abs() < 1e-9 && (vs.out - 1.62).abs() < 1e-9,
            "{vs:?}"
        );
        assert!((t.cues["hi"] - (demo.voice_start + 1.1 - 0.22)).abs() < 1e-3);
        // The asset beat takes the asset's length; the end card its length:.
        let title = &t.beats[2];
        assert!((title.end - title.start - 4.0).abs() < 0.04);
        assert!((t.beats[3].end - t.beats[3].start - 20.0).abs() < 0.04);
        // Boundaries are on the frame grid and back to back.
        for w in t.beats.windows(2) {
            assert_eq!(w[0].end, w[1].start);
            let f = w[0].end * 30.0;
            assert!((f - f.round()).abs() < 1e-6, "{f}");
        }
        assert_eq!(t.beats[3].end, t.duration);
    }

    #[test]
    fn lead_moves_the_picture_boundary_and_length_pads() {
        let sb = parse(SB, &["sprout".to_string()]).unwrap();
        let reads = inputs_reads();
        let mut takes = BTreeMap::new();
        takes.insert("A001".to_string(), ("takes/A001.mp4".to_string(), 60.0));
        let inputs = LockInputs {
            reads: &reads,
            takes: &takes,
            vo: &vo,
        };
        let (base, _) = lock(&sb, &cfg(), &Cuts::default(), &assets(), &inputs).unwrap();
        let mut cuts = Cuts::default();
        cuts.beats.insert(
            "demo".into(),
            BeatCut {
                lead: Some(0.5),
                length: None,
            },
        );
        cuts.beats.insert(
            "hook".into(),
            BeatCut {
                lead: None,
                length: Some(1.0),
            },
        );
        cuts.lines.insert(
            "hook.2".into(),
            LineCut {
                in_: None,
                out: Some(-0.2),
                take: None,
            },
        );
        let (t, _) = lock(&sb, &cfg(), &cuts, &assets(), &inputs).unwrap();
        let hook_voice = base.beats[0].voice_end - 0.2;
        assert!((t.beats[0].voice_end - hook_voice).abs() < 1e-3);
        // The demo's voice starts after the padded hook; its picture 0.5 s later.
        let demo = &t.beats[1];
        assert!((demo.voice_start - (hook_voice + 1.0)).abs() < 1e-3);
        assert!((demo.start - to_frame(demo.voice_start + 0.5, 30)).abs() < 1e-6);
        assert_eq!(t.beats[0].end, demo.start);
    }

    #[test]
    fn lock_names_every_missing_read_and_audio() {
        let sb = parse(SB, &["sprout".to_string()]).unwrap();
        let reads = BTreeMap::new();
        let takes = BTreeMap::new();
        let none = |_: &Line| None;
        let inputs = LockInputs {
            reads: &reads,
            takes: &takes,
            vo: &none,
        };
        let e = lock(&sb, &cfg(), &Cuts::default(), &Assets::new(), &inputs).unwrap_err();
        let j = e.join("\n");
        assert!(j.contains("hook.1 has no complete read"), "{j}");
        assert!(j.contains("hook.2 has no complete read"), "{j}");
        assert!(j.contains("demo.1 (voice sprout) has no audio"), "{j}");
        assert!(j.contains("asset \"logo\", which isn't registered"), "{j}");
    }

    #[test]
    fn estimates_from_words_per_minute() {
        let sb = parse(SB, &["sprout".to_string()]).unwrap();
        let none = |_: &Line| None;
        let (t, w) = estimate(&sb, &cfg(), &Cuts::default(), &Assets::new(), 120.0, &none);
        // 4 words at 120 wpm = 2 s, twice, plus one 0.25 s pause.
        assert!((t.beats[0].voice_end - 4.25).abs() < 1e-9);
        assert!(!t.locked);
        assert!(w.iter().any(|m| m.contains("assuming 4 s")));
        assert_eq!(t.inputs, body_hash(&t));
    }
}
