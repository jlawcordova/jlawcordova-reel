use clap::{Parser, Subcommand};
use cozy_score::analysis::{band_levels, BandLevel};
use cozy_score::dsp::{gain_to_db, SR};
use cozy_score::loudness::{gated, k_power};
use cozy_score::plan::{load_plan_arg, parse_instrument_arg, parse_plan};
use cozy_score::render::{render, wav_bytes, MasterRef, Request};
use cozy_score::resolve::candidates;
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
        /// Plan JSON file, or an inline JSON array.
        #[arg(long)]
        plan: String,
        /// Output directory.
        #[arg(long, default_value = "out")]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = Feel::Spring)]
        feel: Feel,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Comma-separated instruments allowed for the whole score (default: all).
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
    },
    /// List the video types.
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
        } => cmd_render(
            &plan,
            &out,
            feel,
            seed,
            instruments.as_deref(),
            stems,
            mp3,
            gain_from.as_deref(),
        ),
        Cmd::Types => {
            cmd_types();
            Ok(())
        }
        Cmd::Instruments => {
            cmd_instruments();
            Ok(())
        }
        Cmd::Analyze { wav, cue } => cmd_analyze(&wav, &cue),
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

#[allow(clippy::too_many_arguments)]
fn cmd_render(
    plan_arg: &str,
    out: &Path,
    feel: Feel,
    seed: u64,
    instruments: Option<&str>,
    stems: bool,
    mp3: bool,
    gain_from: Option<&Path>,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let specs = match load_plan_arg(plan_arg) {
        Ok(text) => parse_plan(&text).map_err(|e| errors.extend(e)).ok(),
        Err(e) => {
            errors.push(e);
            None
        }
    };
    let score = match instruments {
        None => Some(Instrument::ALL.to_vec()),
        Some(s) => parse_instrument_arg(s).map_err(|e| errors.extend(e)).ok(),
    };
    let master = match gain_from {
        None => None,
        Some(p) => match read_master(p) {
            Ok(m) => Some(m),
            Err(e) => {
                errors.push(e);
                None
            }
        },
    };
    let (Some(specs), Some(score)) = (specs, score) else {
        return Err(errors);
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    if score.is_empty() {
        eprintln!("warning: --instruments is empty; sections without their own list are silent");
    }
    let t0 = std::time::Instant::now();
    let r = render(&Request {
        specs: &specs,
        feel,
        seed,
        instruments: score,
        stems,
        gain_from: master,
    })?;
    for n in &r.notes {
        eprintln!("{n}");
    }
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
    if stems {
        let dir = out.join("stems");
        std::fs::create_dir_all(&dir)
            .map_err(|e| vec![format!("cannot create {}: {e}", dir.display())])?;
        for (id, st) in &r.stems {
            write(dir.join(format!("{id}.wav")), &wav_bytes(st))?;
        }
    }
    let cue = serde_json::to_string_pretty(&r.cue).expect("cue serialises");
    write(out.join("cue.json"), cue.as_bytes())?;
    if mp3 {
        let wavs: Vec<PathBuf> = written
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .cloned()
            .collect();
        encode_mp3s(&wavs);
    }
    let m = &r.cue.master;
    eprintln!(
        "rendered {:.1} s at {} BPM in {:.1} s: {} LUFS, peak {:.2} dBFS, gain {:+.2} dB -> {}",
        r.cue.duration_s,
        r.cue.bpm,
        t0.elapsed().as_secs_f64(),
        m.integrated_lufs
            .map_or("n/a".into(), |l| format!("{l:.2}")),
        m.sample_peak_dbfs,
        m.gain_db,
        out.display()
    );
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
}

fn cmd_instruments() {
    println!("{:<12} description", "instrument");
    for i in Instrument::ALL {
        println!("{:<12} {}", i.id(), i.description());
    }
    println!();
    println!(
        "{:<12} {:<12} {:<26} meaning",
        "role", "preferred", "fallbacks (in order)"
    );
    for r in Role::ALL {
        let (pref, fb) = if r == Role::Bass {
            ("heavy-bass*".to_string(), "soft-bass*".to_string())
        } else {
            let c = candidates(r, Instrument::HeavyBass);
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
    println!();
    println!("* bass prefers soft-bass in summer and in sponsor, recap-cta and reflect; the other bass is the fallback.");
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
