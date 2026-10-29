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
