//! Rendering (spec §7.4, §7.11, §7.16, §9): each beat's picture to its own
//! file with the mode's final codec settings, concatenated with stream
//! copy, then muxed with the mix.

use crate::audio::{self, Buf};
use crate::files::{AssetAudio, AssetEntry, AssetFit, Assets};
use crate::media::{self, SR};
use crate::pixellab::{self, CueKey, Export, PixelLab};
use crate::project::{Errors, Project};
use crate::storyboard::{Beat, Picture, SceneFit, Storyboard};
use crate::timeline::{TBeat, Timeline};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codec {
    H264,
    Prores,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Preview,
    Final,
    Animatic,
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::Preview => "preview",
            Mode::Final => "final",
            Mode::Animatic => "animatic",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub mode: Mode,
    pub size: (u32, u32),
    pub fps: u32,
    pub codec: Codec,
}

impl Settings {
    fn overlay(&self) -> bool {
        self.mode != Mode::Final
    }
    fn ext(&self) -> &'static str {
        match self.codec {
            Codec::H264 => "mp4",
            Codec::Prores => "mov",
        }
    }
    fn video_args(&self) -> Vec<String> {
        let v: &[&str] = match (self.mode, self.codec) {
            (Mode::Final, Codec::H264) => &[
                "-c:v",
                "libx264",
                "-preset",
                "slow",
                "-crf",
                "16",
                "-profile:v",
                "high",
                "-pix_fmt",
                "yuv420p",
            ],
            (Mode::Final, Codec::Prores) => &[
                "-c:v",
                "prores_ks",
                "-profile:v",
                "3",
                "-pix_fmt",
                "yuv422p10le",
            ],
            _ => &[
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "26",
                "-pix_fmt",
                "yuv420p",
            ],
        };
        let mut a: Vec<String> = v.iter().map(|s| s.to_string()).collect();
        a.extend([
            "-r".into(),
            self.fps.to_string(),
            "-video_track_timescale".into(),
            "90000".into(),
        ]);
        a
    }
    fn audio_args(&self) -> Vec<String> {
        let v: &[&str] = match (self.mode, self.codec) {
            (Mode::Final, Codec::Prores) => &["-c:a", "pcm_s24le", "-ar", "48000"],
            (Mode::Final, Codec::H264) => &["-c:a", "aac", "-b:a", "384k", "-ar", "48000"],
            _ => &["-c:a", "aac", "-b:a", "192k", "-ar", "48000"],
        };
        v.iter().map(|s| s.to_string()).collect()
    }
}

/// Everything a beat render needs.
pub struct Ctx<'a> {
    pub p: &'a Project,
    pub sb: &'a Storyboard,
    pub t: &'a Timeline,
    pub assets: &'a Assets,
    pub settings: Settings,
    /// Take name → (path, sha256).
    pub takes: BTreeMap<String, (PathBuf, String)>,
    pub font: Option<String>,
    pub pixellab: Option<PixelLab>,
    pub doc_hash: Option<String>,
    pub warnings: Vec<String>,
}

impl<'a> Ctx<'a> {
    pub fn new(
        p: &'a Project,
        sb: &'a Storyboard,
        t: &'a Timeline,
        assets: &'a Assets,
        settings: Settings,
    ) -> Result<Ctx<'a>, Errors> {
        // Beats are paired by position with the timeline's, so they must be
        // the same beats.
        let ids = |v: Vec<&str>| v.join(",");
        if ids(sb.beats.iter().map(|b| b.id.as_str()).collect())
            != ids(t.beats.iter().map(|b| b.id.as_str()).collect())
        {
            return Err(vec![
                "the storyboard's beats changed since the timeline was locked; run pixelreel cut unlock, then cut lock".into(),
            ]);
        }
        let takes = p
            .takes()?
            .takes
            .into_iter()
            .map(|e| (e.name.clone(), (p.resolve(&e.path), e.sha256)))
            .collect();
        let font = media::font_file();
        let mut warnings = Vec::new();
        if font.is_none() {
            warnings.push(
                "no font found with fc-match; text on cards and labels uses ffmpeg's default"
                    .into(),
            );
        }
        let pixellab = p.pixellab_document().map(|document| PixelLab {
            bin: p.config.pixellab.bin.clone(),
            document,
        });
        Ok(Ctx {
            p,
            sb,
            t,
            assets,
            settings,
            takes,
            font,
            pixellab,
            doc_hash: None,
            warnings,
        })
    }

    fn dir(&self) -> PathBuf {
        self.p.build().join("render").join(self.settings.mode.id())
    }

    fn doc_hash(&mut self) -> Result<String, String> {
        if let Some(h) = &self.doc_hash {
            return Ok(h.clone());
        }
        let doc = self
            .pixellab
            .as_ref()
            .map(|x| x.document.clone())
            .ok_or("scene beats need pixellab.document in pixelreel.yaml")?;
        if !doc.is_dir() {
            return Err(format!(
                "the Pixel Lab document {} doesn't exist",
                doc.display()
            ));
        }
        let h = crate::hash::dir_sha256(&doc)?;
        self.doc_hash = Some(h.clone());
        Ok(h)
    }
}

/// A filter graph under construction: inputs and chains.
#[derive(Default)]
struct Graph {
    inputs: Vec<OsString>,
    n: usize,
    chains: Vec<String>,
    labels: usize,
}

impl Graph {
    fn input(&mut self, pre: &[String], path: &Path) -> usize {
        self.inputs.extend(pre.iter().map(OsString::from));
        self.inputs.push("-i".into());
        self.inputs.push(path.as_os_str().to_owned());
        self.n += 1;
        self.n - 1
    }
    fn lavfi(&mut self, spec: &str) -> usize {
        self.inputs.extend([
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            OsString::from(spec),
        ]);
        self.n += 1;
        self.n - 1
    }
    fn label(&mut self) -> String {
        self.labels += 1;
        format!("v{}", self.labels)
    }
}

/// What a beat shows, worked out before any rendering (also its cache key).
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Pic {
    Face {
        /// (take, source start, frames)
        segments: Vec<(String, f64, u64)>,
    },
    Scene {
        export: ExportKey,
        cues: Vec<(String, String, u64)>,
    },
    Asset {
        id: String,
        sha256: String,
        fit: AssetFit,
        alpha: bool,
        under: Option<Box<Pic>>,
    },
    Card {
        text: String,
    },
}

#[derive(Clone, Debug, Serialize)]
struct ExportKey {
    name: String,
    from_ms: u64,
    to_ms: u64,
    entrance: bool,
}

fn frames_of(t: f64, fps: u32) -> u64 {
    (t * f64::from(fps)).round().max(0.0) as u64
}

fn card_text(beat: &Beat) -> String {
    let mut s = format!(
        "{}  ·  {}  ·  {}\n\n",
        beat.id,
        beat.kind.id(),
        beat.picture.label()
    );
    for l in &beat.lines {
        let who = match &l.voice {
            crate::storyboard::Voice::You => String::new(),
            crate::storyboard::Voice::Vo(v) => format!("{v}: "),
        };
        let mut row = String::new();
        for w in format!("{who}{}", l.text).split_whitespace() {
            if !row.is_empty() && row.chars().count() + 1 + w.chars().count() > 52 {
                s.push_str(&row);
                s.push('\n');
                row.clear();
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(w);
        }
        s.push_str(&row);
        s.push('\n');
    }
    s
}

/// Works out a beat's picture. In the animatic, pictures that need media
/// that may not exist yet fall back to cards.
fn plan_picture(ctx: &mut Ctx, beat: &Beat, tb: &TBeat, picture: &Picture) -> Result<Pic, String> {
    let fps = ctx.settings.fps;
    let animatic = ctx.settings.mode == Mode::Animatic;
    let span = tb.end - tb.start;
    match picture {
        Picture::Card => Ok(Pic::Card {
            text: card_text(beat),
        }),
        Picture::Face if animatic => Ok(Pic::Card {
            text: card_text(beat),
        }),
        Picture::Face => {
            // This beat's own lines with sources, covering the picture span.
            let lines: Vec<_> = tb
                .lines
                .iter()
                .filter(|l| l.source.as_ref().is_some_and(|s| s.take.is_some()))
                .collect();
            if lines.is_empty() {
                return Err(format!(
                    "beat {} shows face but has no recorded lines",
                    beat.id
                ));
            }
            let mut segs = Vec::new();
            let start_f = frames_of(tb.start, fps);
            let end_f = frames_of(tb.end, fps);
            for (i, l) in lines.iter().enumerate() {
                let src = l.source.as_ref().unwrap();
                let seg_start = if i == 0 {
                    tb.start
                } else {
                    l.start.max(tb.start)
                };
                let seg_end = lines.get(i + 1).map_or(tb.end, |n| n.start.min(tb.end));
                let (a, b) = (
                    frames_of(seg_start, fps).max(start_f),
                    frames_of(seg_end, fps).min(end_f),
                );
                if b <= a {
                    continue;
                }
                let offset = src.in_ + (a as f64 / f64::from(fps) - l.start);
                segs.push((src.take.clone().unwrap(), offset.max(0.0), b - a));
            }
            if segs.is_empty() {
                return Err(format!(
                    "beat {} has no picture time left for its lines",
                    beat.id
                ));
            }
            Ok(Pic::Face { segments: segs })
        }
        Picture::Scene(name) => {
            if ctx.pixellab.is_none() {
                if animatic {
                    ctx.warnings.push(format!(
                        "beat {}: no pixellab.document set; showing a card",
                        beat.id
                    ));
                    return Ok(Pic::Card {
                        text: card_text(beat),
                    });
                }
                return Err("scene beats need pixellab.document in pixelreel.yaml".into());
            }
            let from_ms = (beat.from * 1000.0).round() as u64;
            let len_ms = (span * 1000.0).round() as u64;
            let mut to_ms = from_ms + len_ms;
            if beat.fit == SceneFit::Hold {
                let pl = ctx.pixellab.as_ref().unwrap();
                let doc = pl.document.clone();
                let period = pl
                    .period_ms(&doc, name, "render")
                    .map_err(|e| format!("beat {}: {e}", beat.id))?;
                let one = if pixellab::is_sequence(name) {
                    (period - from_ms as f64).max(100.0)
                } else {
                    period
                };
                to_ms = from_ms + (one.round() as u64).clamp(1, len_ms.max(1));
            }
            let mut cues = Vec::new();
            for c in &beat.cues {
                match ctx.t.cues.get(&c.cue) {
                    Some(&at) if at >= tb.start && at <= tb.end => {
                        let ms = from_ms + ((at - tb.start) * 1000.0).round() as u64;
                        cues.push((c.item.clone(), c.state.clone(), ms));
                    }
                    Some(&at) => ctx.warnings.push(format!(
                        "beat {}: cue {} is at {:.2} s, outside the beat's picture ({:.2}–{:.2} s); left out",
                        beat.id, c.cue, at, tb.start, tb.end
                    )),
                    None => ctx.warnings.push(format!("beat {}: cue {} has no time; left out", beat.id, c.cue)),
                }
            }
            Ok(Pic::Scene {
                export: ExportKey {
                    name: name.clone(),
                    from_ms,
                    to_ms,
                    entrance: beat.entrance,
                },
                cues,
            })
        }
        Picture::Asset(id) => {
            let Some(a) = ctx.assets.get(id).cloned() else {
                if animatic {
                    return Ok(Pic::Card {
                        text: card_text(beat),
                    });
                }
                return Err(format!(
                    "beat {} shows asset {id}, which isn't registered (pixelreel asset add)",
                    beat.id
                ));
            };
            let under = match (&beat.under, a.alpha) {
                (Some(u), true) => Some(Box::new(plan_picture(ctx, beat, tb, u)?)),
                (Some(_), false) => {
                    ctx.warnings.push(format!(
                        "beat {}: asset {id} has no alpha, so under: isn't shown",
                        beat.id
                    ));
                    None
                }
                (None, true) => None,
                (None, false) => None,
            };
            Ok(Pic::Asset {
                id: id.clone(),
                sha256: a.sha256.clone(),
                fit: a.fit,
                alpha: a.alpha,
                under,
            })
        }
    }
}

/// Adds a picture's inputs and chains to the graph; returns its label,
/// exactly `frames` frames at the output size and rate.
fn build(
    ctx: &mut Ctx,
    g: &mut Graph,
    pic: &Pic,
    frames: u64,
    span: f64,
    beat_id: &str,
) -> Result<String, String> {
    let (w, h) = ctx.settings.size;
    let fps = ctx.settings.fps;
    let tail = format!(
        "setsar=1,tpad=stop_mode=clone:stop={frames},trim=end_frame={frames},setpts=PTS-STARTPTS"
    );
    match pic {
        Pic::Face { segments } => {
            let mut parts = Vec::new();
            for (take, src, n) in segments {
                let (path, _) = ctx
                    .takes
                    .get(take)
                    .cloned()
                    .ok_or_else(|| format!("take {take} isn't in takes.yaml"))?;
                let secs = *n as f64 / f64::from(fps) + 1.0;
                let i = g.input(
                    &[
                        "-ss".into(),
                        format!("{src:.6}"),
                        "-t".into(),
                        format!("{secs:.6}"),
                    ],
                    &path,
                );
                let l = g.label();
                g.chains.push(format!(
                    "[{i}:v]setpts=PTS-STARTPTS,fps={fps},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1,tpad=stop_mode=clone:stop={n},trim=end_frame={n},setpts=PTS-STARTPTS[{l}]"
                ));
                parts.push(l);
            }
            let l = g.label();
            let ins: String = parts.iter().map(|p| format!("[{p}]")).collect();
            g.chains
                .push(format!("{ins}concat=n={}:v=1:a=0,{tail}[{l}]", parts.len()));
            Ok(l)
        }
        Pic::Scene { export, cues } => {
            let doc_hash = ctx.doc_hash()?;
            let pl = ctx
                .pixellab
                .as_ref()
                .ok_or("scene beats need pixellab.document")?;
            let key = crate::hash::sha256(
                json!({"doc": doc_hash, "export": export, "cues": cues, "size": [w, h], "fps": fps,
                       "bg": ctx.p.config.pixellab.background, "theme": ctx.p.config.pixellab.theme})
                .to_string()
                .as_bytes(),
            )[..16]
                .to_string();
            let base = ctx.p.build().join("pixellab");
            let frames_dir = base.join("frames").join(&key);
            if !frames_dir.join(".done").exists() {
                let mut document = pl.document.clone();
                if !cues.is_empty() {
                    let copy = base.join("docs").join(format!("{key}.pixellab"));
                    let keys: Vec<CueKey> = cues
                        .iter()
                        .map(|(item, state, ms)| CueKey {
                            item: item.clone(),
                            state: state.clone(),
                            time_ms: *ms,
                        })
                        .collect();
                    let w = pl.cue_copy(&export.name, &keys, export.to_ms, &copy, "render")?;
                    ctx.warnings.extend(w);
                    document = copy;
                }
                let e = Export {
                    name: export.name.clone(),
                    from_ms: export.from_ms,
                    to_ms: export.to_ms.max(export.from_ms + 1),
                    entrance: export.entrance,
                    fit: (w, h),
                    fps,
                    background: ctx.p.config.pixellab.background.clone(),
                    theme: ctx.p.config.pixellab.theme.clone(),
                };
                pl.export_frames(&document, &e, &frames_dir, "render")
                    .map_err(|err| format!("beat {beat_id}: {err}"))?;
                std::fs::write(frames_dir.join(".done"), b"").map_err(|err| err.to_string())?;
            }
            let i = g.input(
                &[
                    "-framerate".into(),
                    fps.to_string(),
                    "-start_number".into(),
                    "1".into(),
                ],
                &frames_dir.join("%04d.png"),
            );
            let l = g.label();
            g.chains.push(format!(
                "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease:flags=neighbor,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,{tail}[{l}]"
            ));
            Ok(l)
        }
        Pic::Asset {
            id,
            fit,
            alpha,
            under,
            ..
        } => {
            let a: AssetEntry = ctx
                .assets
                .get(id)
                .cloned()
                .ok_or_else(|| format!("asset {id} isn't registered"))?;
            let path = ctx.p.resolve(&a.path);
            let i = g.input(&[], &path);
            let speed = match fit {
                AssetFit::Speed if a.duration > 0.0 => {
                    format!("setpts={:.6}*(PTS-STARTPTS),", span / a.duration)
                }
                _ => "setpts=PTS-STARTPTS,".into(),
            };
            let (aw, ah) = crate::config::parse_size(&a.size).unwrap_or((w, h));
            let whole = aw > 0 && ah > 0 && w % aw == 0 && h % ah == 0 && w / aw == h / ah;
            let flags = if whole { "neighbor" } else { "lanczos" };
            let fmt = if *alpha { "format=rgba," } else { "" };
            let l = g.label();
            let pad_color = if *alpha { "black@0" } else { "black" };
            g.chains.push(format!(
                "[{i}:v]{speed}fps={fps},{fmt}scale={w}:{h}:force_original_aspect_ratio=decrease:flags={flags},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={pad_color},{tail}[{l}]"
            ));
            match under {
                Some(u) => {
                    let ul = build(ctx, g, u, frames, span, beat_id)?;
                    let o = g.label();
                    g.chains
                        .push(format!("[{ul}][{l}]overlay=format=auto,{tail}[{o}]"));
                    Ok(o)
                }
                None => Ok(l),
            }
        }
        Pic::Card { text } => {
            let secs = frames as f64 / f64::from(fps) + 1.0;
            let i = g.lavfi(&format!("color=c=0x1d2433:s={w}x{h}:r={fps}:d={secs:.3}"));
            let l = g.label();
            let file = ctx.dir().join("cards").join(format!("{beat_id}.txt"));
            crate::files::write_atomic(&file, text.as_bytes()).map_err(|e| e.join("; "))?;
            let size = (h / 24).max(12);
            let dt = drawtext(ctx, &format!(
                "textfile={}:expansion=none:fontsize={size}:fontcolor=white:line_spacing={}:x=(w-text_w)/2:y=(h-text_h)/2",
                lvl2(&file.to_string_lossy()),
                size / 3
            ));
            g.chains.push(format!("[{i}:v]{dt},{tail}[{l}]"));
            Ok(l)
        }
    }
}

/// Escapes a value for a filter's option list (inside the graph's quotes).
fn lvl2(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// A drawtext filter with the found font, quoted for the graph.
fn drawtext(ctx: &Ctx, opts: &str) -> String {
    let font = ctx
        .font
        .as_ref()
        .map(|f| format!("fontfile={}:", lvl2(f)))
        .unwrap_or_default();
    format!("drawtext='{font}{opts}'")
}

/// Renders one beat's picture to its cached file and returns the path.
fn render_beat(ctx: &mut Ctx, beat: &Beat, tb: &TBeat) -> Result<PathBuf, String> {
    let fps = ctx.settings.fps;
    let frames = frames_of(tb.end, fps)
        .saturating_sub(frames_of(tb.start, fps))
        .max(1);
    let span = frames as f64 / f64::from(fps);
    let pic = plan_picture(ctx, beat, tb, &beat.picture)?;
    if ctx.settings.mode == Mode::Final && matches!(pic, Pic::Card { .. }) {
        return Err(format!(
            "beat {} is still a card; render --final needs a real picture (face, scene: or asset:)",
            beat.id
        ));
    }
    // The cache key: the picture, its content hashes and the settings.
    let mut content = json!({});
    collect_hashes(ctx, &pic, &mut content)?;
    let key = json!({
        "pic": pic, "content": content, "frames": frames, "start": tb.start,
        "size": [ctx.settings.size.0, ctx.settings.size.1], "fps": fps,
        "codec": ctx.settings.codec, "mode": ctx.settings.mode,
        "overlay": ctx.settings.overlay(), "label": format!("{} · {}", beat.id, beat.kind.id()),
        "font": ctx.font, "v": 1,
    });
    let hash = &crate::hash::sha256(key.to_string().as_bytes())[..12];
    let out = ctx
        .dir()
        .join("beats")
        .join(format!("{}-{hash}.{}", beat.id, ctx.settings.ext()));
    if out.exists() {
        return Ok(out);
    }
    let mut g = Graph::default();
    let mut label = build(ctx, &mut g, &pic, frames, span, &beat.id)?;
    if ctx.settings.overlay() {
        let file = ctx
            .dir()
            .join("cards")
            .join(format!("{}.label.txt", beat.id));
        crate::files::write_atomic(
            &file,
            format!(
                "{} · {} · {}",
                beat.id,
                beat.kind.id(),
                beat.picture.label()
            )
            .as_bytes(),
        )
        .map_err(|e| e.join("; "))?;
        let size = (ctx.settings.size.1 / 30).max(10);
        let a = drawtext(ctx, &format!(
            "textfile={}:expansion=none:fontsize={size}:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=6:x=12:y=12",
            lvl2(&file.to_string_lossy())
        ));
        let b = drawtext(ctx, &format!(
            "text={}:fontsize={size}:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=6:x=w-text_w-12:y=12",
            lvl2(&format!("%{{pts:hms:{:.3}}}", tb.start))
        ));
        let l = g.label();
        g.chains.push(format!("[{label}]{a},{b}[{l}]"));
        label = l;
    }
    let tmp = out.with_extension(format!("tmp.{}", ctx.settings.ext()));
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut cmd = media::ffmpeg();
    cmd.args(&g.inputs)
        .arg("-filter_complex")
        .arg(g.chains.join(";"))
        .args([
            "-map",
            &format!("[{label}]"),
            "-frames:v",
            &frames.to_string(),
            "-an",
        ])
        .args(ctx.settings.video_args())
        .arg(&tmp);
    crate::proc::run(&mut cmd, &format!("render beat {}", beat.id))?;
    std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn collect_hashes(ctx: &mut Ctx, pic: &Pic, out: &mut Value) -> Result<(), String> {
    match pic {
        Pic::Face { segments } => {
            for (take, _, _) in segments {
                let sha = ctx.takes.get(take).map(|t| t.1.clone()).unwrap_or_default();
                out[format!("take:{take}")] = json!(sha);
            }
        }
        Pic::Scene { .. } => {
            out["pixellab"] = json!(ctx.doc_hash()?);
        }
        Pic::Asset {
            id, sha256, under, ..
        } => {
            out[format!("asset:{id}")] = json!(sha256);
            if let Some(u) = under {
                collect_hashes(ctx, u, out)?;
            }
        }
        Pic::Card { .. } => {}
    }
    Ok(())
}

/// The mix for `[from, to)` of the timeline (spec §9.2). Returns the audio
/// and the voice spans used for ducking.
pub fn mix(
    ctx: &mut Ctx,
    music: Option<&Path>,
    from: f64,
    to: f64,
) -> Result<(Buf, audio::Master), String> {
    let mixc = ctx.p.config.mix;
    let n = ((to - from) * f64::from(SR)).round().max(1.0) as usize;
    let mut bus = Buf::silent(n);
    let fade = (0.010 * f64::from(SR)) as usize;
    let mut spans = Vec::new();
    let place = |t: f64| -> usize { ((t - from) * f64::from(SR)).round().max(0.0) as usize };
    // Voice.
    for line in ctx.t.lines() {
        let Some(src) = &line.source else { continue };
        if line.end <= from || line.start >= to {
            continue;
        }
        let path = match &src.take {
            Some(take) => ctx
                .takes
                .get(take)
                .map(|t| t.0.clone())
                .ok_or_else(|| format!("take {take} isn't in takes.yaml"))?,
            None => ctx.p.resolve(&src.file),
        };
        let (l, r) = media::decode_audio(
            &path,
            src.in_,
            Some(src.out - src.in_),
            &format!("mix {}", line.id),
        )?;
        let mut b = Buf { l, r };
        audio::fade_edges(&mut b, fade);
        if line.start >= from {
            bus.add_at(&b, place(line.start), 1.0);
        } else {
            let skip = ((from - line.start) * f64::from(SR)) as usize;
            let tail = Buf {
                l: b.l.get(skip..).unwrap_or(&[]).to_vec(),
                r: b.r.get(skip..).unwrap_or(&[]).to_vec(),
            };
            bus.add_at(&tail, 0, 1.0);
        }
        spans.push((line.start - from, line.end - from));
    }
    // Asset audio.
    for (beat, tb) in ctx.sb.beats.iter().zip(&ctx.t.beats) {
        let Picture::Asset(id) = &beat.picture else {
            continue;
        };
        let Some(a) = ctx.assets.get(id) else {
            continue;
        };
        if !a.has_audio || a.audio == AssetAudio::Mute || tb.end <= from || tb.start >= to {
            continue;
        }
        let len = tb.end - tb.start;
        if a.fit == AssetFit::Speed && (a.duration - len).abs() > 0.05 {
            ctx.warnings.push(format!(
                "beat {}: asset {id} is retimed, so its audio is left out",
                beat.id
            ));
            continue;
        }
        let (l, r) = media::decode_audio(
            &ctx.p.resolve(&a.path),
            0.0,
            Some(len.min(a.duration)),
            &format!("mix asset {id}"),
        )?;
        let mut b = Buf { l, r };
        audio::fade_edges(&mut b, fade);
        if tb.start >= from {
            bus.add_at(&b, place(tb.start), 1.0);
        }
    }
    // Music, ducked under the voice.
    if let Some(m) = music {
        let (l, r) = media::decode_audio(m, from, Some(to - from), "mix music")?;
        let env = audio::duck_envelope(
            l.len(),
            &spans,
            mixc.duck_db,
            mixc.duck_attack_ms,
            mixc.duck_release_ms,
            SR,
        );
        let mut b = Buf { l, r };
        for (i, &e) in env.iter().enumerate() {
            b.l[i] *= e;
            b.r[i] *= e;
        }
        bus.add_at(&b, 0, 1.0);
    }
    let m = audio::master(&mut bus, mixc.target_lufs, mixc.true_peak_dbtp, SR);
    Ok((bus, m))
}

#[derive(Serialize)]
pub struct Report {
    pub pixelreel: String,
    pub mode: Mode,
    pub file: String,
    pub size: String,
    pub fps: u32,
    pub frames: u64,
    pub duration: f64,
    pub timeline: String,
    pub score: Option<Value>,
    pub master: audio::Master,
    pub beats: Vec<Value>,
    pub captions: usize,
    pub chapters: Vec<(f64, String)>,
    pub warnings: Vec<String>,
}

/// Concatenates beat files with stream copy and muxes the mix in.
fn assemble(ctx: &Ctx, beat_files: &[PathBuf], mix_wav: &Path, out: &Path) -> Result<(), String> {
    let dir = out.parent().unwrap_or(Path::new("."));
    let list = dir.join("concat.txt");
    let body: String = beat_files
        .iter()
        .map(|f| format!("file '{}'\n", f.to_string_lossy().replace('\'', r"'\''")))
        .collect();
    std::fs::write(&list, body).map_err(|e| e.to_string())?;
    let video_only = dir.join(format!("video-only.{}", ctx.settings.ext()));
    crate::proc::run(
        media::ffmpeg()
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&list)
            .args(["-c", "copy"])
            .arg(&video_only),
        "render (joining beats)",
    )?;
    let tmp = out.with_extension(format!("tmp.{}", ctx.settings.ext()));
    let mut cmd = media::ffmpeg();
    cmd.arg("-i")
        .arg(&video_only)
        .arg("-i")
        .arg(mix_wav)
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy"])
        .args(ctx.settings.audio_args());
    if ctx.settings.codec == Codec::H264 {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(&tmp);
    crate::proc::run(&mut cmd, "render (mux)")?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&list);
    let _ = std::fs::remove_file(&video_only);
    Ok(())
}

/// Renders the whole timeline. Returns the report.
pub fn render_all(
    ctx: &mut Ctx,
    music: Option<(&Path, Value)>,
    out: &Path,
) -> Result<Report, Errors> {
    let mut errors = Vec::new();
    let mut files = Vec::new();
    let mut beats = Vec::new();
    let sb = ctx.sb;
    let t = ctx.t;
    for (beat, tb) in sb.beats.iter().zip(&t.beats) {
        match render_beat(ctx, beat, tb) {
            Ok(f) => {
                beats.push(json!({
                    "id": beat.id, "type": tb.kind, "picture": tb.picture,
                    "start": tb.start, "end": tb.end,
                    "file": f.file_name().map(|n| n.to_string_lossy().into_owned()),
                }));
                files.push(f);
            }
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let (mixed, master) =
        mix(ctx, music.as_ref().map(|m| m.0), 0.0, t.duration).map_err(|e| vec![e])?;
    let mix_wav = out.with_file_name("mix.wav");
    audio::write_wav(&mix_wav, &mixed, SR).map_err(|e| vec![e])?;
    drop(mixed);
    assemble(ctx, &files, &mix_wav, out).map_err(|e| vec![e])?;
    let _ = std::fs::remove_file(&mix_wav);
    let probe = media::probe(out, "render").map_err(|e| vec![e])?;
    let frames: u64 = t
        .beats
        .iter()
        .map(|b| frames_of(b.end, t.fps) - frames_of(b.start, t.fps))
        .sum();
    let caps = crate::captions::captions(t);
    let dir = out.parent().unwrap_or(Path::new("."));
    crate::files::write_atomic(
        &dir.join("captions.srt"),
        crate::captions::srt(&caps).as_bytes(),
    )?;
    crate::files::write_atomic(
        &dir.join("chapters.txt"),
        crate::captions::chapters_txt(t).as_bytes(),
    )?;
    let report = Report {
        pixelreel: crate::timeline::VERSION.into(),
        mode: ctx.settings.mode,
        file: ctx.p.stored_path(out),
        size: format!("{}x{}", ctx.settings.size.0, ctx.settings.size.1),
        fps: t.fps,
        frames,
        duration: crate::time::to_ms(probe.duration),
        timeline: t.inputs.clone(),
        score: music.map(|m| m.1),
        master,
        beats,
        captions: caps.len(),
        chapters: crate::captions::chapters(t),
        warnings: ctx.warnings.clone(),
    };
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    crate::files::write_atomic(
        &dir.join("render-report.json"),
        format!("{text}\n").as_bytes(),
    )?;
    Ok(report)
}

/// `scene preview <beat>`: one beat with its voice, at preview settings.
pub fn preview_beat(ctx: &mut Ctx, beat_id: &str, out: &Path) -> Result<(), Errors> {
    let sb = ctx.sb;
    let t = ctx.t;
    let i = sb
        .beats
        .iter()
        .position(|b| b.id == beat_id)
        .ok_or_else(|| vec![format!("no beat \"{beat_id}\"")])?;
    let (beat, tb) = (&sb.beats[i], &t.beats[i]);
    let file = render_beat(ctx, beat, tb).map_err(|e| vec![e])?;
    let (mixed, _) = mix(ctx, None, tb.start, tb.end).map_err(|e| vec![e])?;
    let mix_wav = out.with_file_name(format!("{beat_id}.mix.wav"));
    audio::write_wav(&mix_wav, &mixed, SR).map_err(|e| vec![e])?;
    assemble(ctx, &[file], &mix_wav, out).map_err(|e| vec![e])?;
    let _ = std::fs::remove_file(&mix_wav);
    Ok(())
}
