//! Same plan + instruments + seed + feel + version => byte-identical WAV.
//!
//! When the sound changes on purpose, update the stored hash deliberately:
//!     COZY_UPDATE_HASH=1 cargo test --test determinism

use cozy_score::plan::parse_plan;
use cozy_score::render::{render, wav_bytes, Request};
use cozy_score::types::{Feel, Instrument};
use sha2::{Digest, Sha256};

const PLAN: &str = include_str!("fixtures/reference-short.json");
const HASH_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/reference-short.sha256"
);

fn render_hash() -> String {
    let specs = parse_plan(PLAN).unwrap();
    let r = render(&Request {
        specs: &specs,
        feel: Feel::Spring,
        seed: 7,
        instruments: Instrument::ALL.to_vec(),
        stems: false,
        gain_from: None,
    })
    .unwrap();
    let mut h = Sha256::new();
    h.update(wav_bytes(&r.full));
    h.update(wav_bytes(&r.no_melody));
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn reference_render_matches_stored_hash() {
    let got = render_hash();
    if std::env::var("COZY_UPDATE_HASH").is_ok() {
        std::fs::write(HASH_FILE, format!("{got}\n")).unwrap();
        return;
    }
    let want = std::fs::read_to_string(HASH_FILE)
        .expect("stored hash; run with COZY_UPDATE_HASH=1 to create it");
    assert_eq!(
        got,
        want.trim(),
        "render changed; if intended, run COZY_UPDATE_HASH=1 cargo test --test determinism"
    );
}

#[test]
fn rendering_twice_is_identical() {
    assert_eq!(render_hash(), render_hash());
}

// ------------------------------------------------------------------ plans with events

/// Renders a plan file from `plans/` with seed 7, as the CLI does (its own
/// feel, tempo and key; the fade for plans with new features).
fn plan_hash(name: &str) -> String {
    use cozy_score::events::ParseContext;
    use cozy_score::plan::parse_plan_with;
    use cozy_score::render::{render_with, FadeOut, Options};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plans");
    let text = std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap();
    let p = parse_plan_with(
        &text,
        &ParseContext {
            base: &dir,
            document: None,
            seed: 7,
        },
    )
    .unwrap();
    let feel = p.feel.unwrap_or(Feel::Spring);
    let r = render_with(
        &Request {
            specs: &p.sections,
            feel,
            seed: 7,
            instruments: feel.default_instruments(),
            stems: false,
            gain_from: None,
        },
        &Options {
            events: p.events.clone(),
            bpm: p.bpm,
            key: p.key,
            fade_out: FadeOut::Auto,
            tail: 0.0,
            fps: p.fps,
        },
    )
    .unwrap();
    let mut h = Sha256::new();
    h.update(wav_bytes(&r.full));
    h.update(wav_bytes(&r.no_melody));
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn check_plan_hash(name: &str) {
    let file = format!(
        "{}/tests/fixtures/{name}.sha256",
        env!("CARGO_MANIFEST_DIR")
    );
    let got = plan_hash(name);
    if std::env::var("COZY_UPDATE_HASH").is_ok() {
        std::fs::write(&file, format!("{got}\n")).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&file)
        .expect("stored hash; run with COZY_UPDATE_HASH=1 to create it");
    assert_eq!(
        got,
        want.trim(),
        "{name} render changed; if intended, run COZY_UPDATE_HASH=1 cargo test --test determinism"
    );
}

/// The section 1 reference plan: a chip sting with runs, ticks and hits.
#[test]
fn startup_sting_matches_stored_hash() {
    check_plan_hash("startup-sting");
}

/// An entrance scored from pixellab pieces (the parity fixture's hero island).
#[test]
fn entrance_plan_matches_stored_hash() {
    check_plan_hash("entrance-hero");
}
