//! End to end: a fixture project with generated media, a fake whisper, a
//! fake Pixel Lab and a local fake ElevenLabs, through every command (plan
//! "Proof").

use base64::Engine;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pixelreel")
}

fn fakes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fakes")
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg runs");
    assert!(
        out.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Env {
    root: PathBuf,
    project: PathBuf,
    vars: Vec<(String, String)>,
}

impl Env {
    fn run(&self, args: &[&str]) -> Output {
        let mut c = Command::new(bin());
        c.arg("-C").arg(&self.project).args(args);
        for (k, v) in &self.vars {
            c.env(k, v);
        }
        c.output().expect("pixelreel runs")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "pixelreel {args:?} failed:\n{stdout}\n{stderr}"
        );
        format!("{stdout}{stderr}")
    }
}

/// Whisper JSON for words spoken from `t0`, 0.5 s apart.
fn whisper_words(words: &mut Vec<Value>, text: &str, t0: f64) -> f64 {
    let mut t = t0;
    for w in text.split_whitespace() {
        words.push(json!({
            "offsets": {"from": (t * 1000.0).round() as i64, "to": ((t + 0.4) * 1000.0).round() as i64},
            "text": format!(" {w}"),
        }));
        t += 0.5;
    }
    t
}

/// A fake ElevenLabs: answers every text-to-speech request with the same
/// audio and an even character alignment over it.
fn fake_elevenlabs(mp3: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let audio = base64::engine::general_purpose::STANDARD.encode(mp3);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, len) = loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break (None, 0);
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                    assert!(
                        head.contains("xi-api-key: test-key"),
                        "the key is sent as a header"
                    );
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    break (Some(i + 4), len);
                }
            };
            let Some(start) = head_end else { continue };
            while buf.len() < start + len {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body: Value =
                serde_json::from_slice(&buf[start..start + len]).unwrap_or(Value::Null);
            let text = body["text"].as_str().unwrap_or("").to_string();
            let chars: Vec<String> = text.chars().map(|c| c.to_string()).collect();
            let n = chars.len().max(1) as f64;
            let starts: Vec<f64> = (0..chars.len()).map(|i| 0.2 + 1.6 * i as f64 / n).collect();
            let ends: Vec<f64> = (0..chars.len())
                .map(|i| 0.2 + 1.6 * (i + 1) as f64 / n)
                .collect();
            let resp = json!({
                "audio_base64": audio,
                "alignment": {
                    "characters": chars,
                    "character_start_times_seconds": starts,
                    "character_end_times_seconds": ends,
                },
            })
            .to_string();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}",
                resp.len()
            );
        }
    });
    format!("http://{addr}")
}

const STORYBOARD: &str = "---
title: Test Devlog
---

## hook | cold-open-hook | face
chapter: Can a farm run itself?
Thirty days ago this field was totally empty.
Today it plants waters and harvests itself.

## tease | cold-open-hook | scene:farm
cue lights: lamp-1 on
And I have not touched it in a whole week{#lights}.

## title | title-sting | asset:logo

## demo | visual-demo | scene:farm
chapter: Day one
fit: hold
length: 10s
VO(sprout): Day one. Soil quality questionable.

## quote | reflect | asset:quote
under: scene:farm
VO(sprout): The best fertilizer is the farmer's footsteps.

## recap | recap-cta | face
chapter: Recap
length: 10s
That is the whole farm so far. Subscribe for day thirty one.

## outro | outro-end-screen | asset:end
";

fn setup() -> Env {
    let root = std::env::temp_dir().join(format!("pixelreel-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let project = root.join("proj");
    let r = |p: &str| root.join(p).to_string_lossy().into_owned();

    // Media.
    std::fs::create_dir_all(root.join("media")).unwrap();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=640x360:r=30:d=40",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:d=40",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-c:a",
        "aac",
        "-shortest",
        &r("media/A001.mp4"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=640x360:r=30:d=4",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=660:d=4",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-c:a",
        "aac",
        "-shortest",
        &r("media/logo.mp4"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=red@0.5:s=320x180:r=30:d=3,format=rgba",
        "-c:v",
        "prores_ks",
        "-profile:v",
        "4",
        "-pix_fmt",
        "yuva444p10le",
        &r("media/quote.mov"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=s=640x360:r=30:d=8",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        &r("media/end.mp4"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=220:d=2.2",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "128k",
        &r("media/vo.mp3"),
    ]);

    // A take with a false start, the lines, an ad-lib and a retake.
    let mut words = Vec::new();
    let mut t = 1.0;
    t = whisper_words(&mut words, "thirty days ago this", t) + 1.0;
    t = whisper_words(
        &mut words,
        "thirty days ago this field was totally empty",
        t,
    ) + 0.2;
    t = whisper_words(&mut words, "today it plants waters and harvests itself", t) + 1.0;
    t = whisper_words(&mut words, "and i have not touched it in a whole week", t) + 1.0;
    t = whisper_words(
        &mut words,
        "hold on let me check the light again before the next one",
        t,
    ) + 1.0;
    t = whisper_words(
        &mut words,
        "that is the whole farm so far subscribe for day thirty",
        t,
    ) + 1.0;
    let end = whisper_words(
        &mut words,
        "that is the whole farm so far subscribe for day thirty one",
        t,
    );
    assert!(end < 40.0, "the take is long enough: {end}");
    std::fs::create_dir_all(root.join("whisper")).unwrap();
    std::fs::write(
        root.join("whisper/A001.json"),
        json!({"transcription": words}).to_string(),
    )
    .unwrap();

    // A Pixel Lab document (the fake only copies it).
    std::fs::create_dir_all(root.join("art/farm.pixellab/scenes")).unwrap();
    std::fs::write(root.join("art/farm.pixellab/manifest.json"), "{}").unwrap();
    std::fs::write(root.join("art/farm.pixellab/scenes/farm.json"), "{}").unwrap();
    std::fs::write(root.join("model.bin"), "fake model").unwrap();

    let out = Command::new(bin())
        .args(["init"])
        .arg(&project)
        .args(["--size", "640x360", "--fps", "30", "--document"])
        .arg(root.join("art/farm.pixellab"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = std::fs::read_to_string(project.join("pixelreel.yaml")).unwrap();
    let cfg = cfg
        .replace(
            "  bin: pixellab\n",
            &format!("  bin: {}\n", fakes().join("pixellab").display()),
        )
        .replace(
            "  bin: whisper-cli\n",
            &format!(
                "  bin: {}\n  model: {}\n",
                fakes().join("whisper-cli").display(),
                r("model.bin")
            ),
        )
        .replace(
            "voices: {}\n",
            "voices:\n  sprout:\n    voice_id: robot-voice\n",
        );
    std::fs::write(project.join("pixelreel.yaml"), cfg).unwrap();
    std::fs::write(project.join("storyboard.md"), STORYBOARD).unwrap();
    let mp3 = std::fs::read(root.join("media/vo.mp3")).unwrap();
    let url = fake_elevenlabs(mp3);
    let vars = vec![
        ("FAKE_WHISPER_DIR".to_string(), r("whisper")),
        ("FAKE_PIXELLAB_LOG".to_string(), r("pixellab.log")),
        ("ELEVENLABS_API_KEY".to_string(), "test-key".to_string()),
        ("PIXELREEL_ELEVENLABS_URL".to_string(), url),
        ("NO_PROXY".to_string(), "127.0.0.1,localhost".to_string()),
        ("no_proxy".to_string(), "127.0.0.1,localhost".to_string()),
    ];
    Env {
        root,
        project,
        vars,
    }
}

fn probe_duration(path: &Path) -> f64 {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn the_whole_pipeline() {
    let e = setup();
    let p = &e.project;

    // Pre-production: the board estimates, the score sketches, the assets.
    let board = e.ok(&["board"]);
    assert!(board.contains("≈"), "{board}");
    assert!(board.contains("2 VO line(s) (2 not generated)"), "{board}");
    let logo = e.root.join("media/logo.mp4");
    let quote = e.root.join("media/quote.mov");
    let end = e.root.join("media/end.mp4");
    e.ok(&["asset", "add", logo.to_str().unwrap(), "--id", "logo"]);
    e.ok(&[
        "asset",
        "add",
        quote.to_str().unwrap(),
        "--id",
        "quote",
        "--alpha",
        "--fit",
        "hold-last",
    ]);
    e.ok(&[
        "asset",
        "add",
        end.to_str().unwrap(),
        "--id",
        "end",
        "--fit",
        "hold-last",
    ]);
    let bad = e.run(&[
        "asset",
        "add",
        logo.to_str().unwrap(),
        "--id",
        "bad",
        "--alpha",
    ]);
    assert!(
        !bad.status.success(),
        "a file without alpha is refused for --alpha"
    );
    let sketch = e.ok(&["score", "render", "--seeds", "7"]);
    assert!(sketch.contains("seed 7:"), "{sketch}");
    let scenes = e.ok(&["scene", "list"]);
    assert!(scenes.contains("farm"), "{scenes}");

    // ElevenLabs lines, cached.
    let vo = e.ok(&["vo", "gen", "--takes", "2"]);
    assert_eq!(vo.matches("take ").count(), 4, "{vo}");
    let again = e.ok(&["vo", "gen", "--takes", "2"]);
    assert!(again.contains("nothing to do"), "{again}");
    e.ok(&["vo", "pick", "quote", "2"]);

    // The take: transcribed, aligned, the last complete read used.
    let take = e.ok(&[
        "take",
        "add",
        e.root.join("media/A001.mp4").to_str().unwrap(),
    ]);
    assert!(take.contains("hook.1"), "{take}");
    assert!(take.contains("2 read(s), 1 complete"), "{take}");
    assert!(take.contains("ad-lib in A001"), "{take}");

    // Cut and lock.
    e.ok(&[
        "cut",
        "tighten",
        "--max-gap",
        "0.3",
        "--keep-breath",
        "0.12",
    ]);
    e.ok(&["cut", "set", "recap.1", "--out", "-0.05"]);
    e.ok(&["cut", "lead", "tease", "0.3"]);
    e.ok(&["cut", "lock"]);
    let locked1 = std::fs::read_to_string(p.join("build/timeline.json")).unwrap();
    e.ok(&["cut", "unlock"]);
    e.ok(&["cut", "lock"]);
    let locked2 = std::fs::read_to_string(p.join("build/timeline.json")).unwrap();
    assert_eq!(locked1, locked2, "the same inputs give the same timeline");
    let refused = e.run(&["cut", "lead", "demo", "0.2"]);
    assert!(
        !refused.status.success(),
        "cut edits are refused while locked"
    );
    let t: Value = serde_json::from_str(&locked2).unwrap();
    let duration = t["duration"].as_f64().unwrap();
    assert!(duration > 40.0 && duration < 70.0, "{duration}");
    // The tease beat's picture starts 0.3 s after its voice.
    let tease = t["beats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "tease")
        .unwrap();
    assert!(
        (tease["start"].as_f64().unwrap() - tease["voice_start"].as_f64().unwrap() - 0.3).abs()
            < 0.02,
        "{tease}"
    );
    assert!(t["cues"]["lights"].as_f64().is_some());
    let show = e.ok(&["cut", "show", "hook"]);
    assert!(show.contains("A001"), "{show}");

    // Music from the locked edit.
    e.ok(&["score", "render", "--seeds", "3,7"]);
    e.ok(&["score", "pick", "7"]);
    let status = e.ok(&["status"]);
    assert!(status.contains("timeline: locked\n"), "{status}");
    assert!(status.contains("music: seed 7\n"), "{status}");

    // Preview, one beat, final, and the YouTube check.
    let preview = e.ok(&["render", "--preview"]);
    assert!(preview.contains("preview 960x540"), "{preview}");
    let pv = p.join("build/render/preview/video.mp4");
    assert!(
        (probe_duration(&pv) - duration).abs() < 0.1,
        "{} vs {duration}",
        probe_duration(&pv)
    );
    e.ok(&["scene", "preview", "demo"]);
    assert!(p.join("build/preview/demo.mp4").exists());
    let fin = e.ok(&["render", "--final", "--size", "640x360"]);
    assert!(fin.contains("final 640x360"), "{fin}");
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(p.join("build/render/final/render-report.json")).unwrap(),
    )
    .unwrap();
    let lufs = report["master"]["integrated_lufs"].as_f64().unwrap();
    assert!((lufs + 14.0).abs() <= 0.5, "{lufs}");
    assert!(report["master"]["true_peak_dbtp"].as_f64().unwrap() <= -1.0 + 1e-6);
    let fv = p.join("build/render/final/video.mp4");
    assert!((probe_duration(&fv) - duration).abs() < 0.1);
    let chapters = std::fs::read_to_string(p.join("build/render/final/chapters.txt")).unwrap();
    assert!(
        chapters.starts_with("0:00 Can a farm run itself?\n"),
        "{chapters}"
    );
    assert_eq!(chapters.lines().count(), 3);
    let srt = std::fs::read_to_string(p.join("build/render/final/captions.srt")).unwrap();
    assert!(
        srt.contains("Thirty days ago this field was totally"),
        "{srt}"
    );
    let check = e.ok(&["check", "--youtube"]);
    assert!(check.contains("✓ chapters"), "{check}");
    assert!(check.contains("✓ loudness"), "{check}");
    assert!(check.contains("✓ media"), "{check}");

    // Pixel Lab was asked for frames at the output size, and the cue keyed
    // the lamp on a copy of the document.
    let log = std::fs::read_to_string(e.root.join("pixellab.log")).unwrap();
    assert!(log.contains("art export farm"), "{log}");
    assert!(log.contains("--fit 640,360"), "{log}");
    assert!(log.contains("scene key set farm lamp-1 --time"), "{log}");
    assert!(log.contains("--state on"), "{log}");
    assert!(
        !log.contains(&format!(
            "--document {} scene key",
            e.root.join("art/farm.pixellab").display()
        )),
        "the owner's document is never edited"
    );

    // A re-render reuses every beat.
    let started = std::time::Instant::now();
    e.ok(&["render", "--preview"]);
    assert!(started.elapsed().as_secs_f64() < 60.0);

    // The animatic, from estimates.
    e.ok(&["animatic"]);
    assert!(p.join("build/animatic.mp4").exists());

    if std::env::var_os("PIXELREEL_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&e.root);
    }
}
