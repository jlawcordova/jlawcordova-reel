//! cozy-score: generates cozy farm-sim style background music from a video plan.
//!
//! Pipeline of pure stages: parse -> validate -> plan (timing) -> arrangement
//! (note events by role) -> instrument resolution (allow-lists, fallbacks) ->
//! synthesis -> mix -> master -> encode.

pub mod analysis;
pub mod arrange;
pub mod dsp;
pub mod harmony;
pub mod loudness;
pub mod melody;
pub mod mix;
pub mod par;
pub mod plan;
pub mod render;
pub mod resolve;
pub mod rng;
pub mod synth;
pub mod timing;
pub mod types;
