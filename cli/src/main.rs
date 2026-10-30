use clap::{Args, Parser, Subcommand};
use pixelreel::files::{AssetAudio, AssetFit};
use pixelreel::project::{Errors, Project};
use pixelreel::render::{self, Codec, Mode, Settings};
use pixelreel::storyboard::{Picture, Voice};
use pixelreel::time::{clock, parse_offset};
use pixelreel::{assets, cuts, score, status, takes, vo};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "pixelreel",
    version,
    about = "Storyboard, takes, ElevenLabs lines, Pixel Lab scenes and cozy-score music, rendered as one video"
)]
struct Cli {
    /// The project folder.
    #[arg(short = 'C', global = true, default_value = ".")]
    dir: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Make a new project folder.
    Init {
        dir: PathBuf,
        #[arg(long, default_value = "1920x1080")]
        size: String,
        #[arg(long, default_value_t = 30)]
        fps: u32,
        #[arg(long, default_value = "spring")]
        feel: String,
        /// The Pixel Lab document (.pixellab) scene beats come from.
        #[arg(long)]
        document: Option<String>,
    },
    /// Check the storyboard and show its beats with their times.
    Board {
        #[arg(long)]
        json: bool,
    },
    /// Render a rough cut from the storyboard alone (cards, scenes, estimates).
    Animatic {
        /// Voice every line with the first voice in pixelreel.yaml.
        #[arg(long)]
        tts: bool,
    },
    /// ElevenLabs lines.
    #[command(subcommand)]
    Vo(VoCmd),
    /// Talking-head recordings.
    #[command(subcommand)]
    Take(TakeCmd),
    /// Timing edits and the lock.
    #[command(subcommand)]
    Cut(CutCmd),
    /// Pixel Lab scenes.
    #[command(subcommand)]
    Scene(SceneCmd),
    /// Outside animations.
    #[command(subcommand)]
    Asset(AssetCmd),
    /// Music from cozy-score.
    #[command(subcommand)]
    Score(ScoreCmd),
    /// Render the video.
    Render(RenderArgs),
    /// What's done and what's out of date, beat by beat.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Check the final render against YouTube's rules.
    Check {
        #[arg(long)]
        youtube: bool,
        /// Warnings fail too.
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum VoCmd {
    /// Generate VO lines (all, or in the named beats).
    Gen {
        beats: Vec<String>,
        #[arg(long)]
        voice: Option<String>,
        #[arg(long, default_value_t = 1)]
        takes: u32,
        #[arg(long)]
        force: bool,
    },
    /// Choose a take for a line (<beat>.<n>) or every VO line in a beat.
    Pick { target: String, take: u32 },
}

#[derive(Subcommand)]
enum TakeCmd {
    /// Transcribe recordings and find each line's reads.
    Add {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CutCmd {
    /// Each line's take, in/out points, times and gaps.
    Show {
        beat: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Set the join rules between lines.
    Tighten {
        #[arg(long, allow_hyphen_values = true)]
        max_gap: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        keep_breath: Option<String>,
    },
    /// Nudge a line's in/out (<beat>.<n>) or a beat's length.
    Set {
        target: String,
        #[arg(long = "in", allow_hyphen_values = true)]
        in_: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        out: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        length: Option<String>,
    },
    /// Use another take's read of a line.
    Take { line: String, take: String },
    /// Start a beat's picture this many seconds after its voice (J-cut).
    Lead { beat: String, seconds: String },
    /// Freeze exact times into build/timeline.json.
    Lock,
    /// Remove the lock.
    Unlock,
}

#[derive(Subcommand)]
enum SceneCmd {
    /// Scenes and sequences in the Pixel Lab document.
    List,
    /// Render one beat with its locked voice to build/preview/<beat>.mp4.
    Preview { beat: String },
}

#[derive(Subcommand)]
enum AssetCmd {
    /// Register an outside animation.
    Add {
        file: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long, value_enum, default_value_t = AssetFit::Trim)]
        fit: AssetFit,
        #[arg(long)]
        alpha: bool,
        #[arg(long, value_enum, default_value_t = AssetAudio::Keep)]
        audio: AssetAudio,
    },
}

#[derive(Subcommand)]
enum ScoreCmd {
    /// Write build/score/plan.json.
    Plan,
    /// Render music with cozy-score, one seed after another.
    Render {
        /// Comma-separated seeds (default: the picked seed, or 7).
        #[arg(long)]
        seeds: Option<String>,
        #[arg(long)]
        instruments: Option<String>,
        #[arg(long)]
        stems: bool,
    },
    /// Choose a rendered seed.
    Pick { seed: u64 },
}

#[derive(Args)]
struct RenderArgs {
    #[arg(long, conflicts_with = "final_")]
    preview: bool,
    #[arg(long = "final")]
    final_: bool,
    #[arg(long)]
    size: Option<String>,
    #[arg(long, value_enum, default_value_t = Codec::H264)]
    codec: Codec,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

fn warn(ws: &[String]) {
    for w in ws {
        eprintln!("warning: {w}");
    }
}

fn print(lines: &[String]) {
    for l in lines {
        println!("{l}");
    }
}

fn secs(arg: Option<&str>, name: &str) -> Result<Option<f64>, Errors> {
    match arg {
        None => Ok(None),
        Some(s) => parse_offset(s)
            .map(Some)
            .ok_or_else(|| vec![format!("{name} \"{s}\" must be seconds, like 0.3 or -0.2")]),
    }
}

fn run(cli: Cli) -> Result<ExitCode, Errors> {
    if let Cmd::Init {
        dir,
        size,
        fps,
        feel,
        document,
    } = &cli.cmd
    {
        pixelreel::project::init(dir, size, *fps, feel, document.as_deref())?;
        println!(
            "made {}; write storyboard.md, then run pixelreel -C {} board",
            dir.display(),
            dir.display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    let p = Project::open(&cli.dir)?;
    let sb = p.storyboard()?;
    match cli.cmd {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Board { json } => board(&p, &sb, json)?,
        Cmd::Animatic { tts } => animatic(&p, &sb, tts)?,
        Cmd::Vo(VoCmd::Gen {
            beats,
            voice,
            takes,
            force,
        }) => {
            let (out, w) = vo::generate(&p, &sb, &beats, voice.as_deref(), takes, force)?;
            print(&out);
            warn(&w);
        }
        Cmd::Vo(VoCmd::Pick { target, take }) => print(&vo::pick(&p, &sb, &target, take)?),
        Cmd::Take(TakeCmd::Add { files }) => {
            let (out, w) = takes::add(&p, &files)?;
            print(&out);
            warn(&w);
        }
        Cmd::Cut(c) => cut(&p, &sb, c)?,
        Cmd::Scene(SceneCmd::List) => scene_list(&p, &sb)?,
        Cmd::Scene(SceneCmd::Preview { beat }) => {
            let t = cuts::locked(&p).ok_or_else(|| {
                vec!["scene preview needs a locked timeline; run pixelreel cut lock".to_string()]
            })?;
            let assets = p.assets()?;
            let settings = Settings {
                mode: Mode::Preview,
                size: (960, 540),
                fps: p.config.fps,
                codec: Codec::H264,
            };
            let mut ctx = render::Ctx::new(&p, &sb, &t, &assets, settings)?;
            let out = p.build().join("preview").join(format!("{beat}.mp4"));
            std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| vec![e.to_string()])?;
            render::preview_beat(&mut ctx, &beat, &out)?;
            warn(&ctx.warnings);
            println!("{}", p.stored_path(&out));
        }
        Cmd::Asset(AssetCmd::Add {
            file,
            id,
            fit,
            alpha,
            audio,
        }) => {
            print(&assets::add(&p, &file, &id, fit, alpha, audio)?);
        }
        Cmd::Score(s) => score_cmd(&p, &sb, s)?,
        Cmd::Render(r) => render_cmd(&p, &sb, r)?,
        Cmd::Status { json } => {
            let s = status::status(&p, &sb)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
            } else {
                print(&status::status_text(&s));
            }
        }
        Cmd::Check {
            youtube,
            strict,
            json,
        } => {
            if !youtube {
                return Err(vec!["say what to check: pixelreel check --youtube".into()]);
            }
            let checks = status::check_youtube(&p, &sb)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&checks).unwrap_or_default()
                );
            } else {
                for c in &checks {
                    let mark = match c.level.as_str() {
                        "pass" => "✓",
                        "warn" => "⚠",
                        _ => "✗",
                    };
                    println!("{mark} {:<13} {}", c.name, c.detail);
                }
            }
            let failed = checks
                .iter()
                .any(|c| c.level == "fail" || (strict && c.level == "warn"));
            return Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            });
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn voices_label(b: &pixelreel::storyboard::Beat) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for l in &b.lines {
        let k = l.voice.label().to_string();
        match counts.iter_mut().find(|c| c.0 == k) {
            Some(c) => c.1 += 1,
            None => counts.push((k, 1)),
        }
    }
    if counts.is_empty() {
        return "–".into();
    }
    counts
        .iter()
        .map(|(k, n)| format!("{k} ×{n}"))
        .collect::<Vec<_>>()
        .join(" + ")
}

fn board(
    p: &Project,
    sb: &pixelreel::storyboard::Storyboard,
    json_out: bool,
) -> Result<(), Errors> {
    let (t, warnings) = cuts::current(p, sb)?;
    let picks = p.picks()?;
    if json_out {
        let beats: Vec<_> = sb
            .beats
            .iter()
            .zip(&t.beats)
            .map(|(b, tb)| {
                json!({"id": b.id, "type": b.kind.id(), "picture": b.picture.label(), "voice": voices_label(b),
                       "start": tb.start, "end": tb.end, "chapter": b.chapter})
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"title": sb.title, "locked": t.locked, "duration": t.duration, "beats": beats, "warnings": warnings}))
                .unwrap_or_default()
        );
        return Ok(());
    }
    let w_id = sb
        .beats
        .iter()
        .map(|b| b.id.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let w_pic = sb
        .beats
        .iter()
        .map(|b| b.picture.label().chars().count())
        .max()
        .unwrap_or(7)
        .max(7);
    let w_voice = sb
        .beats
        .iter()
        .map(|b| voices_label(b).chars().count())
        .max()
        .unwrap_or(5)
        .max(5);
    let col = if t.locked { "locked" } else { "est" };
    println!(
        "{:>2}  {:<w_id$}  {:<16}  {:<w_pic$}  {:<w_voice$}  {:>6}  {:>6}",
        "#", "beat", "type", "picture", "voice", col, "at"
    );
    for (i, (b, tb)) in sb.beats.iter().zip(&t.beats).enumerate() {
        println!(
            "{:>2}  {:<w_id$}  {:<16}  {:<w_pic$}  {:<w_voice$}  {:>6}  {:>6}",
            i + 1,
            b.id,
            b.kind.id(),
            b.picture.label(),
            voices_label(b),
            clock(tb.end - tb.start),
            clock(tb.start)
        );
    }
    let you = sb
        .beats
        .iter()
        .flat_map(|b| &b.lines)
        .filter(|l| l.voice == Voice::You)
        .count();
    let vo_lines: Vec<_> = sb
        .beats
        .iter()
        .flat_map(|b| &b.lines)
        .filter(|l| matches!(l.voice, Voice::Vo(_)))
        .collect();
    let missing = vo_lines
        .iter()
        .filter(|l| vo::picked_audio(p, l, &picks).is_none())
        .count();
    println!(
        "total {} {}  {you} line(s) you, {} VO line(s){}",
        if t.locked { "=" } else { "≈" },
        clock(t.duration),
        vo_lines.len(),
        if missing > 0 {
            format!(" ({missing} not generated)")
        } else {
            String::new()
        }
    );
    warn(&warnings);
    Ok(())
}

fn cut(p: &Project, sb: &pixelreel::storyboard::Storyboard, c: CutCmd) -> Result<(), Errors> {
    let msg = match c {
        CutCmd::Show { beat, json } => {
            let (t, w) = cuts::current(p, sb)?;
            if let Some(b) = &beat {
                if t.beat(b).is_none() {
                    return Err(vec![format!("no beat \"{b}\"")]);
                }
            }
            if json {
                let beats: Vec<_> = t
                    .beats
                    .iter()
                    .filter(|b| beat.as_deref().is_none_or(|x| x == b.id))
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&beats).unwrap_or_default()
                );
            } else {
                print(&cuts::show(&t, beat.as_deref()));
            }
            warn(&w);
            return Ok(());
        }
        CutCmd::Tighten {
            max_gap,
            keep_breath,
        } => cuts::tighten(
            p,
            secs(max_gap.as_deref(), "--max-gap")?,
            secs(keep_breath.as_deref(), "--keep-breath")?,
        )?,
        CutCmd::Set {
            target,
            in_,
            out,
            length,
        } => cuts::set(
            p,
            sb,
            &target,
            secs(in_.as_deref(), "--in")?,
            secs(out.as_deref(), "--out")?,
            secs(length.as_deref(), "--length")?,
        )?,
        CutCmd::Take { line, take } => cuts::take(p, sb, &line, &take)?,
        CutCmd::Lead { beat, seconds } => {
            let s = secs(Some(&seconds), "seconds")?.unwrap_or(0.0);
            cuts::lead(p, sb, &beat, s)?
        }
        CutCmd::Lock => {
            let (out, w) = cuts::lock(p, sb)?;
            print(&out);
            warn(&w);
            return Ok(());
        }
        CutCmd::Unlock => cuts::unlock(p)?,
    };
    println!("{msg}");
    Ok(())
}

fn scene_list(p: &Project, sb: &pixelreel::storyboard::Storyboard) -> Result<(), Errors> {
    let doc = p
        .pixellab_document()
        .ok_or_else(|| vec!["set pixellab.document in pixelreel.yaml".to_string()])?;
    let pl = pixelreel::pixellab::PixelLab {
        bin: p.config.pixellab.bin.clone(),
        document: doc,
    };
    let (scenes, seqs) = pl.list("scene list").map_err(|e| vec![e])?;
    println!(
        "scenes:    {}",
        if scenes.is_empty() {
            "none".into()
        } else {
            scenes.join(", ")
        }
    );
    println!(
        "sequences: {}",
        if seqs.is_empty() {
            "none".into()
        } else {
            seqs.join(", ")
        }
    );
    let mut missing = Vec::new();
    for b in &sb.beats {
        for pic in std::iter::once(&b.picture).chain(b.under.iter()) {
            if let Picture::Scene(n) = pic {
                if !scenes.contains(n) && !seqs.contains(n) {
                    missing.push(format!(
                        "beat {} names scene:{n}, which the document doesn't have",
                        b.id
                    ));
                }
            }
        }
    }
    warn(&missing);
    Ok(())
}

fn score_cmd(
    p: &Project,
    sb: &pixelreel::storyboard::Storyboard,
    s: ScoreCmd,
) -> Result<(), Errors> {
    match s {
        ScoreCmd::Plan => {
            let (t, w) = cuts::current(p, sb)?;
            let plan = score::write_plan(p, &t)?;
            println!(
                "{} sections from {} times -> build/score/plan.json",
                plan.as_array().map_or(0, Vec::len),
                if t.locked { "locked" } else { "estimated" }
            );
            for sec in plan.as_array().into_iter().flatten() {
                println!(
                    "  {:<18} {}–{}",
                    sec[0].as_str().unwrap_or(""),
                    clock(sec[1][0].as_f64().unwrap_or(0.0)),
                    clock(sec[1][1].as_f64().unwrap_or(0.0))
                );
            }
            warn(&w);
        }
        ScoreCmd::Render {
            seeds,
            instruments,
            stems,
        } => {
            let (t, mut w) = cuts::current(p, sb)?;
            let seeds: Vec<u64> = match seeds {
                Some(s) => s
                    .split(',')
                    .map(|x| {
                        x.trim()
                            .parse()
                            .map_err(|_| vec![format!("seed \"{x}\" isn't a whole number")])
                    })
                    .collect::<Result<_, _>>()?,
                None => vec![score::picked(p)?.unwrap_or(7)],
            };
            let (out, w2) = score::render(p, &t, &seeds, instruments.as_deref(), stems)?;
            print(&out);
            w.extend(w2);
            warn(&w);
        }
        ScoreCmd::Pick { seed } => {
            score::pick(p, seed)?;
            println!("music: seed {seed}");
        }
    }
    Ok(())
}

fn music_for(
    p: &Project,
    t: &pixelreel::timeline::Timeline,
    need_pick: bool,
    stale_fails: bool,
    w: &mut Vec<String>,
) -> Result<Option<(PathBuf, serde_json::Value)>, Errors> {
    let (seed, stale) = status::score_state(p, t)?;
    let Some(seed) = seed else {
        if need_pick {
            return Err(vec![
                "pick a score first: pixelreel score render --seeds 3,7,11, then score pick <seed>"
                    .into(),
            ]);
        }
        w.push("no score picked; rendering without music".into());
        return Ok(None);
    };
    if stale {
        let msg = format!("the picked score (seed {seed}) was rendered from another plan; run pixelreel score render --seeds {seed}");
        if stale_fails {
            return Err(vec![msg]);
        }
        w.push(msg);
    }
    let path = score::seed_dir(p, seed).join("full.wav");
    if !path.exists() {
        return Err(vec![format!(
            "seed {seed} has no full.wav; run pixelreel score render --seeds {seed}"
        )]);
    }
    let meta = score::seed_meta(p, seed);
    Ok(Some((
        path,
        json!({"seed": seed, "plan_hash": meta.map(|m| m.plan_hash)}),
    )))
}

fn render_cmd(
    p: &Project,
    sb: &pixelreel::storyboard::Storyboard,
    r: RenderArgs,
) -> Result<(), Errors> {
    if r.preview == r.final_ {
        return Err(vec!["say --preview or --final".into()]);
    }
    let mode = if r.final_ { Mode::Final } else { Mode::Preview };
    let t = cuts::locked(p).ok_or_else(|| {
        vec!["render needs a locked timeline; run pixelreel cut lock".to_string()]
    })?;
    let mut w = Vec::new();
    if cuts::locked_is_stale(p, sb) == Some(true) {
        let msg = "the locked timeline is out of date; run pixelreel cut unlock, then cut lock"
            .to_string();
        if mode == Mode::Final {
            return Err(vec![msg]);
        }
        w.push(msg);
    }
    let music = music_for(p, &t, true, mode == Mode::Final, &mut w)?;
    let size = match (&r.size, mode) {
        (Some(s), _) => pixelreel::config::parse_size(s)
            .ok_or_else(|| vec![format!("--size \"{s}\" must be WxH with even numbers")])?,
        (None, Mode::Final) => p.config.size(),
        (None, _) => (960, 540),
    };
    if mode == Mode::Preview && r.codec == Codec::Prores {
        return Err(vec!["--codec prores is for --final".into()]);
    }
    let assets = p.assets()?;
    let settings = Settings {
        mode,
        size,
        fps: t.fps,
        codec: r.codec,
    };
    let ext = if r.codec == Codec::Prores {
        "mov"
    } else {
        "mp4"
    };
    let mut ctx = render::Ctx::new(p, sb, &t, &assets, settings)?;
    ctx.warnings.extend(w);
    let out = p.render_dir(mode.id()).join(format!("video.{ext}"));
    std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| vec![e.to_string()])?;
    let started = std::time::Instant::now();
    let report = render::render_all(
        &mut ctx,
        music.as_ref().map(|(p, v)| (p.as_path(), v.clone())),
        &out,
    )?;
    println!(
        "{} {}x{} {} fps, {}, {} LUFS, {} dBTP, in {:.0} s -> {}",
        mode.id(),
        size.0,
        size.1,
        t.fps,
        clock(report.duration),
        report
            .master
            .integrated_lufs
            .map_or("n/a".into(), |l| format!("{l:.1}")),
        report.master.true_peak_dbtp,
        started.elapsed().as_secs_f64(),
        report.file
    );
    println!(
        "captions: {} cues; chapters: {}",
        report.captions,
        report.chapters.len()
    );
    warn(&report.warnings);
    Ok(())
}

fn animatic(p: &Project, sb: &pixelreel::storyboard::Storyboard, tts: bool) -> Result<(), Errors> {
    let voice = if tts {
        let v = p
            .config
            .voices
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| vec!["--tts needs a voice in pixelreel.yaml".to_string()])?;
        let lines: Vec<_> = sb
            .beats
            .iter()
            .flat_map(|b| b.lines.iter())
            .map(|l| (l, v.clone()))
            .collect();
        let (out, w) = vo::synthesize(p, &lines, 1, false, "animatic --tts")?;
        print(&out);
        warn(&w);
        Some(v)
    } else {
        None
    };
    let (t, mut w) = cuts::estimate(p, sb, voice.as_deref())?;
    let music = music_for(p, &t, false, false, &mut w)?;
    let assets = p.assets()?;
    let settings = Settings {
        mode: Mode::Animatic,
        size: (960, 540),
        fps: p.config.fps,
        codec: Codec::H264,
    };
    let mut ctx = render::Ctx::new(p, sb, &t, &assets, settings)?;
    ctx.warnings.extend(w);
    let dir = p.render_dir("animatic");
    std::fs::create_dir_all(&dir).map_err(|e| vec![e.to_string()])?;
    let out = dir.join("video.mp4");
    let report = render::render_all(
        &mut ctx,
        music.as_ref().map(|(p, v)| (p.as_path(), v.clone())),
        &out,
    )?;
    let dest = p.build().join("animatic.mp4");
    std::fs::rename(&out, &dest).map_err(|e| vec![e.to_string()])?;
    println!(
        "animatic {} -> {}",
        clock(report.duration),
        p.stored_path(Path::new(&dest))
    );
    warn(&report.warnings);
    Ok(())
}
