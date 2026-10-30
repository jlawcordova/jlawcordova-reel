use clap::{Parser, Subcommand};
use cozy_score::analysis::{band_levels, BandLevel};
use cozy_score::detect;
use cozy_score::dsp::{gain_to_db, SR};
use cozy_score::events::ParseContext;
use cozy_score::harmony::parse_key;
use cozy_score::loudness::{gated, k_power};
use cozy_score::pixellab::{draft_from_sequence, SequenceDraft};
use cozy_score::plan::{check_bpm, load_plan_arg, parse_instrument_arg, parse_plan_with};
use cozy_score::render::{render_with, wav_bytes, FadeOut, MasterRef, Options, Request};
use cozy_score::resolve::candidates_in;
use cozy_score::types::{Feel, Instrument, Role, VideoType};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(
    name = "cozy-score",
    version,
    about = "Cozy farm-sim background music that follows a video plan"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render full.wav, no-melody.wav and cue.json from a plan.
    Render {
        /// Plan JSON file, or inline JSON (an array of sections or a plan object).
        #[arg(long)]
        plan: String,
        /// Output directory.
        #[arg(long, default_value = "out")]
        out: PathBuf,
        /// spring, summer or chip (default: the plan's feel, else spring).
        #[arg(long, value_enum)]
        feel: Option<Feel>,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Comma-separated instruments allowed for the whole score (default: the feel's).
        #[arg(long)]
        instruments: Option<String>,
        /// Also write one WAV per bus to <out>/stems/.
        #[arg(long)]
        stems: bool,
        /// Also write a 256 kbps MP3 of each WAV (needs ffmpeg on PATH).
        #[arg(long)]
        mp3: bool,
        /// Reuse the master gain and saturation of an earlier render's cue.json,
        /// so two renders can be compared sample by sample.
        #[arg(long)]
        gain_from: Option<PathBuf>,
        /// An exact tempo, 60-200 (skips the search).
        #[arg(long)]
        bpm: Option<f64>,
        /// A major key: C, C#, Db, ... B (default: the feel's).
        #[arg(long)]
        key: Option<String>,
        /// Seconds of fade-out at the end (default: min(3, 0.1 x length)).
        #[arg(long)]
        fade_out: Option<f64>,
        /// Extra seconds of audio after the last section, so the last hit rings out.
        #[arg(long, default_value_t = 0.0)]
        tail: f64,
        /// Also write with-music.mp4: this video with full.wav (needs ffmpeg).
        #[arg(long)]
        video: Option<PathBuf>,
        /// Align entrance and exit sounds to this frame rate.
        #[arg(long)]
        fps: Option<f64>,
        /// The .pixellab document `scene` sources read (needs pixellab on PATH).
        #[arg(long)]
        pixellab_document: Option<PathBuf>,
    },
    /// List the video types, feels and event kinds.
    Types,
    /// List the instruments, the roles and their fallbacks.
    Instruments,
    /// Measure a rendered WAV per section, using its cue.json.
    Analyze {
        #[arg(long)]
        wav: PathBuf,
        #[arg(long)]
        cue: PathBuf,
    },
    /// Draft a plan from a video's motion (needs ffmpeg on PATH).
    Detect {
        #[arg(long)]
        video: PathBuf,
        /// Write the plan here instead of printing it.
        #[arg(long)]
        out: Option<PathBuf>,
        /// The feel whose tempo range the suggestion prefers.
        #[arg(long, value_enum, default_value_t = Feel::Chip)]
        feel: Feel,
        /// Motion threshold as a share of the clip's 99th-percentile motion.
        #[arg(long, default_value_t = 0.25)]
        threshold: f64,
    },
    /// Draft a plan from a pixellab sequence and its scenes' entrances.
    FromPixellab {
        /// The output of `pixellab sequence show <name> --json`.
        #[arg(long)]
        sequence: PathBuf,
        /// A directory of `pixellab scene entrance show <scene> --json` outputs, as <scene>.json.
        #[arg(long)]
        entrances: PathBuf,
        /// Section types by shot index, as JSON: '{"0":"cold-open-hook","2":"outro-end-screen"}'.
        #[arg(long)]
        types: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, value_enum)]
        feel: Option<Feel>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Render {
            plan,
            out,
            feel,
            seed,
            instruments,
            stems,
            mp3,
            gain_from,
            bpm,
            key,
            fade_out,
            tail,
            video,
            fps,
            pixellab_document,
        } => cmd_render(&RenderArgs {
            plan_arg: &plan,
            out: &out,
            feel,
            seed,
            instruments: instruments.as_deref(),
            stems,
            mp3,
            gain_from: gain_from.as_deref(),
            bpm,
            key: key.as_deref(),
            fade_out,
            tail,
            video: video.as_deref(),
            fps,
            document: pixellab_document.as_deref(),
        }),
        Cmd::Types => {
            cmd_types();
            Ok(())
        }
        Cmd::Instruments => {
            cmd_instruments();
            Ok(())
        }
        Cmd::Analyze { wav, cue } => cmd_analyze(&wav, &cue),
        Cmd::Detect {
            video,
            out,
            feel,
            threshold,
        } => cmd_detect(&video, out.as_deref(), feel, threshold),
        Cmd::FromPixellab {
            sequence,
            entrances,
            types,
            out,
            feel,
        } => cmd_from_pixellab(
            &sequence,
            &entrances,
            types.as_deref(),
            out.as_deref(),
            feel,
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(errors) => {
            for e in errors {
                eprintln!("error: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

struct RenderArgs<'a> {
    plan_arg: &'a str,
    out: &'a Path,
    feel: Option<Feel>,
    seed: u64,
    instruments: Option<&'a str>,
    stems: bool,
    mp3: bool,
    gain_from: Option<&'a Path>,
    bpm: Option<f64>,
    key: Option<&'a str>,
    fade_out: Option<f64>,
    tail: f64,
    video: Option<&'a Path>,
    fps: Option<f64>,
    document: Option<&'a Path>,
}

fn cmd_render(a: &RenderArgs) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    // `from` paths in the plan are relative to the plan file.
    let t = a.plan_arg.trim_start();
    let inline = t.starts_with('[') || t.starts_with('{');
    let base = if inline {
        PathBuf::from(".")
    } else {
        Path::new(a.plan_arg)
            .parent()
            .map(Path::to_path_buf)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let cx = ParseContext {
        base: &base,
        document: a.document,
        seed: a.seed,
    };
    let plan = match load_plan_arg(a.plan_arg) {
        Ok(text) => parse_plan_with(&text, &cx)
            .map_err(|e| errors.extend(e))
            .ok(),
        Err(e) => {
            errors.push(e);
            None
        }
    };
    let feel = a
        .feel
        .or(plan.as_ref().and_then(|p| p.feel))
        .unwrap_or(Feel::Spring);
    let score = match a.instruments {
        None => Some(feel.default_instruments()),
        Some(s) => parse_instrument_arg(s).map_err(|e| errors.extend(e)).ok(),
    };
    let master = match a.gain_from {
        None => None,
        Some(p) => match read_master(p) {
            Ok(m) => Some(m),
            Err(e) => {
                errors.push(e);
                None
            }
        },
    };
    let bpm = match a.bpm {
        Some(b) => check_bpm(b)
            .map_err(|e| errors.push(format!("--bpm: {e}")))
            .ok(),
        None => plan.as_ref().and_then(|p| p.bpm),
    };
    let key = match a.key {
        Some(k) => parse_key(k)
            .map_err(|e| errors.push(format!("--key: {e}")))
            .ok(),
        None => plan.as_ref().and_then(|p| p.key),
    };
    if let Some(f) = a.fade_out {
        if !(f.is_finite() && f >= 0.0) {
            errors.push(format!("--fade-out must be 0 or more seconds, got {f}"));
        }
    }
    if !(a.tail.is_finite() && (0.0..=60.0).contains(&a.tail)) {
        errors.push(format!(
            "--tail must be from 0 to 60 seconds, got {}",
            a.tail
        ));
    }
    if let Some(f) = a.fps {
        if !(1.0..=1000.0).contains(&f) {
            errors.push(format!("--fps must be from 1 to 1000, got {f}"));
        }
    }
    let (Some(plan), Some(score)) = (plan, score) else {
        return Err(errors);
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    if score.is_empty() {
        eprintln!("warning: --instruments is empty; sections without their own list are silent");
    }
    // A bare array plan without any newer option keeps the original fade.
    let classic =
        plan.legacy && feel != Feel::Chip && bpm.is_none() && key.is_none() && a.tail == 0.0;
    let fade_out = match a.fade_out {
        Some(f) => FadeOut::Seconds(f),
        None if classic => FadeOut::Legacy,
        None => FadeOut::Auto,
    };
    let t0 = std::time::Instant::now();
    let r = render_with(
        &Request {
            specs: &plan.sections,
            feel,
            seed: a.seed,
            instruments: score,
            stems: a.stems,
            gain_from: master,
        },
        &Options {
            events: plan.events,
            bpm,
            key,
            fade_out,
            tail: a.tail,
            fps: a.fps.or(plan.fps),
        },
    )?;
    for n in &r.notes {
        eprintln!("{n}");
    }
    let out = a.out;
    std::fs::create_dir_all(out)
        .map_err(|e| vec![format!("cannot create {}: {e}", out.display())])?;
    let mut written: Vec<PathBuf> = Vec::new();
    let mut write = |path: PathBuf, bytes: &[u8]| -> Result<(), Vec<String>> {
        std::fs::write(&path, bytes)
            .map_err(|e| vec![format!("cannot write {}: {e}", path.display())])?;
        written.push(path);
        Ok(())
    };
    write(out.join("full.wav"), &wav_bytes(&r.full))?;
    write(out.join("no-melody.wav"), &wav_bytes(&r.no_melody))?;
    if a.stems {
        let dir = out.join("stems");
        std::fs::create_dir_all(&dir)
            .map_err(|e| vec![format!("cannot create {}: {e}", dir.display())])?;
        for (id, st) in &r.stems {
            write(dir.join(format!("{id}.wav")), &wav_bytes(st))?;
        }
    }
    let cue = serde_json::to_string_pretty(&r.cue).expect("cue serialises");
    write(out.join("cue.json"), cue.as_bytes())?;
    if a.mp3 {
        let wavs: Vec<PathBuf> = written
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .cloned()
            .collect();
        encode_mp3s(&wavs);
    }
    if let Some(v) = a.video {
        mux_video(v, &out.join("full.wav"), &out.join("with-music.mp4"));
    }
    let m = &r.cue.master;
    eprintln!(
        "rendered {:.1} s at {} BPM in {} in {:.1} s: {} LUFS, peak {:.2} dBFS, gain {:+.2} dB -> {}",
        r.cue.audio_duration_s,
        r.cue.bpm,
        r.cue.key,
        t0.elapsed().as_secs_f64(),
        m.integrated_lufs
            .map_or("n/a".into(), |l| format!("{l:.2}")),
        m.sample_peak_dbfs,
        m.gain_db,
        out.display()
    );
    Ok(())
}

/// Writes `out`: the video's stream copied, `wav` as 256 kbps AAC, cut to
/// the video's length.
fn mux_video(video: &Path, wav: &Path, out: &Path) {
    if !detect::have_ffmpeg() {
        eprintln!("warning: --video needs ffmpeg and ffprobe on PATH; skipped with-music.mp4");
        return;
    }
    let (_, dur) = match detect::probe(video) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("warning: --video: {e}; skipped with-music.mp4");
            return;
        }
    };
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(video)
        .arg("-i")
        .arg(wav)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy"])
        .args(["-c:a", "aac", "-b:a", "256k", "-af", "apad"])
        .args(["-t", &format!("{dur:.6}")])
        .arg(out)
        .status();
    match status {
        Ok(s) if s.success() => eprintln!("wrote {} ({dur:.3} s)", out.display()),
        _ => eprintln!("warning: ffmpeg failed to write {}", out.display()),
    }
}

fn cmd_detect(
    video: &Path,
    out: Option<&Path>,
    feel: Feel,
    threshold: f64,
) -> Result<(), Vec<String>> {
    if !(threshold > 0.0 && threshold < 1.0) {
        return Err(vec![format!(
            "--threshold must be between 0 and 1, got {threshold}"
        )]);
    }
    let (motion, fps, duration) = detect::measure(video).map_err(|e| vec![e])?;
    let d = detect::classify(&motion, fps, threshold);
    let plan = detect::draft_plan(&d, duration, feel);
    let text = serde_json::to_string_pretty(&plan).expect("plan serialises");
    match out {
        Some(p) => {
            std::fs::write(p, format!("{text}\n"))
                .map_err(|e| vec![format!("cannot write {}: {e}", p.display())])?;
            eprintln!(
                "{} windows, {} ticks, {} BPM -> {}",
                d.windows.len(),
                d.ticks.len(),
                plan["bpm"],
                p.display()
            );
        }
        None => println!("{text}"),
    }
    Ok(())
}

fn cmd_from_pixellab(
    sequence: &Path,
    entrances: &Path,
    types: Option<&str>,
    out: Option<&Path>,
    feel: Option<Feel>,
) -> Result<(), Vec<String>> {
    let text = std::fs::read_to_string(sequence)
        .map_err(|e| vec![format!("cannot read {}: {e}", sequence.display())])?;
    let seq: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| vec![format!("{} is not JSON: {e}", sequence.display())])?;
    let mut type_list = Vec::new();
    if let Some(t) = types {
        let v: serde_json::Value =
            serde_json::from_str(t).map_err(|e| vec![format!("--types is not JSON: {e}")])?;
        let Some(o) = v.as_object() else {
            return Err(vec![
                "--types must be an object like {\"0\":\"intro\"}".into()
            ]);
        };
        let mut errors = Vec::new();
        for (k, v) in o {
            let i: Option<usize> = k.parse().ok();
            let kind = v.as_str().filter(|s| VideoType::from_id(s).is_some());
            match (i, kind) {
                (Some(i), Some(kind)) => type_list.push((i, kind.to_string())),
                _ => errors.push(format!(
                    "--types: \"{k}\": {v} must map a shot index to a video type (see cozy-score types)"
                )),
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
    }
    let plan_dir = out
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let plan = draft_from_sequence(&SequenceDraft {
        sequence: &seq,
        entrances,
        types: &type_list,
        plan_dir,
        feel: feel.map(Feel::id),
    })?;
    let text = serde_json::to_string_pretty(&plan).expect("plan serialises");
    match out {
        Some(p) => std::fs::write(p, format!("{text}\n"))
            .map_err(|e| vec![format!("cannot write {}: {e}", p.display())])?,
        None => println!("{text}"),
    }
    Ok(())
}

fn read_master(p: &Path) -> Result<MasterRef, String> {
    let text =
        std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", p.display()))?;
    let gain = v["master"]["gain"].as_f64();
    let sat = v["master"]["saturation_peak"].as_f64();
    match (gain, sat) {
        (Some(gain), Some(saturation_peak)) => Ok(MasterRef {
            gain,
            saturation_peak,
        }),
        _ => Err(format!(
            "{} has no master.gain / master.saturation_peak",
            p.display()
        )),
    }
}

fn encode_mp3s(wavs: &[PathBuf]) {
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("warning: --mp3 needs ffmpeg on PATH; skipped MP3 encoding");
        return;
    }
    for w in wavs {
        let mp3 = w.with_extension("mp3");
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(w)
            .args(["-codec:a", "libmp3lame", "-b:a", "256k"])
            .arg(&mp3)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => eprintln!("warning: ffmpeg failed to encode {}", mp3.display()),
        }
    }
}

fn cmd_types() {
    println!("{:<18} {:<11} description", "type", "energy");
    for t in VideoType::ALL {
        println!(
            "{:<18} {:<11} {}",
            t.id(),
            t.energy_label(),
            t.description()
        );
    }
    println!();
    println!("{:<18} description", "feel");
    for f in Feel::ALL {
        println!("{:<18} {}", f.id(), f.description());
    }
    println!();
    println!("{:<18} description", "event");
    for (k, d) in [
        ("run", "a staggered run of notes (start + count + step|end, or times), optional riser and land"),
        ("ticks", "small accents (times, or start + every + count), blip/mallet/clap/shaker, accents"),
        ("hit", "a one-shot accent at `at`: soft, land, pop or big"),
        ("riser", "a noise swell from start to end (or reverse)"),
        ("entrance", "a pixellab scene building up, piece by piece (from, pieces, grid or scene)"),
        ("exit", "a pixellab scene breaking apart: its entrance played backwards"),
    ] {
        println!("{k:<18} {d}");
    }
}

fn cmd_instruments() {
    println!("{:<12} description", "instrument");
    for i in Instrument::ALL {
        println!("{:<12} {}", i.id(), i.description());
    }
    for feel in [Feel::Spring, Feel::Chip] {
        println!();
        println!(
            "{} ({}):",
            if feel == Feel::Chip {
                "chip feel"
            } else {
                "spring and summer feels"
            },
            if feel == Feel::Chip {
                "default instruments: all"
            } else {
                "default instruments: the first nine"
            }
        );
        println!(
            "{:<12} {:<12} {:<26} meaning",
            "role", "preferred", "fallbacks (in order)"
        );
        for r in Role::ALL {
            let (pref, fb) = if r == Role::Bass {
                if feel == Feel::Chip {
                    ("soft-bass".to_string(), "heavy-bass".to_string())
                } else {
                    ("heavy-bass*".to_string(), "soft-bass*".to_string())
                }
            } else {
                let c = candidates_in(r, Instrument::HeavyBass, feel);
                (
                    c[0].id().to_string(),
                    if c.len() > 1 {
                        c[1..].iter().map(|i| i.id()).collect::<Vec<_>>().join(", ")
                    } else {
                        "none (dropped)".into()
                    },
                )
            };
            println!("{:<12} {:<12} {:<26} {}", r.id(), pref, fb, r.description());
        }
    }
    println!();
    println!("* bass prefers soft-bass in summer and in sponsor, recap-cta and reflect; the other bass is the fallback.");
    println!("Events ask for an instrument (a run's `instrument`, a tick's `sound`) and fall back through their role.");
}

fn cmd_analyze(wav: &Path, cue: &Path) -> Result<(), Vec<String>> {
    let mut reader = hound::WavReader::open(wav)
        .map_err(|e| vec![format!("cannot open {}: {e}", wav.display())])?;
    let spec = reader.spec();
    if spec.channels != 2 || spec.bits_per_sample != 16 {
        return Err(vec!["expected a 16-bit stereo WAV".into()]);
    }
    let samples: Vec<f64> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f64 / 32767.0))
        .collect::<Result<_, _>>()
        .map_err(|e| vec![e.to_string()])?;
    let l: Vec<f64> = samples.iter().step_by(2).copied().collect();
    let r: Vec<f64> = samples.iter().skip(1).step_by(2).copied().collect();
    let text = std::fs::read_to_string(cue)
        .map_err(|e| vec![format!("cannot read {}: {e}", cue.display())])?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| vec![e.to_string()])?;
    let kp = k_power(&[&l, &r], SR);
    let peak = l.iter().chain(&r).fold(0.0f64, |a, x| a.max(x.abs()));
    println!(
        "{}: integrated {} LUFS, sample peak {:.2} dBFS",
        wav.display(),
        gated(&kp, SR, 0, kp.len()).map_or("n/a".into(), |x| format!("{x:.2}")),
        gain_to_db(peak.max(1e-12))
    );
    let empty = Vec::new();
    let secs = v["sections"].as_array().unwrap_or(&empty);
    struct Row {
        kind: String,
        start: f64,
        lufs: Option<f64>,
        bands: Vec<BandLevel>,
    }
    let rows: Vec<Row> = secs
        .iter()
        .map(|s| {
            let start = s["start_s"].as_f64().unwrap_or(0.0);
            let a = (start * SR).round() as usize;
            let b = (s["end_s"].as_f64().unwrap_or(0.0) * SR).round() as usize;
            Row {
                kind: s["type"].as_str().unwrap_or("?").to_string(),
                start,
                lufs: gated(&kp, SR, a, b),
                bands: band_levels(&l, &r, a, b),
            }
        })
        .collect();
    let loudest = rows
        .iter()
        .filter_map(|r| r.lufs)
        .fold(f64::NEG_INFINITY, f64::max);
    print!(
        "{:<18} {:>7} {:>7} {:>7} {:>6}",
        "section", "start", "LUFS", "rel LU", ""
    );
    if let Some(r) = rows.first() {
        for b in &r.bands {
            print!(" {:>7}", b.band);
        }
    }
    println!();
    for row in &rows {
        print!(
            "{:<18} {:>7.2} {:>7} {:>7} {:>6}",
            row.kind,
            row.start,
            row.lufs.map_or("n/a".into(), |x| format!("{x:.2}")),
            row.lufs
                .map_or("n/a".into(), |x| format!("{:.2}", x - loudest)),
            "dB:"
        );
        for b in &row.bands {
            print!(" {:>7.1}", b.db);
        }
        println!();
    }
    Ok(())
}
