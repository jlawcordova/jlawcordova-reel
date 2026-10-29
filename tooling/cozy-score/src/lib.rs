//! cozy-score: generates cozy farm-sim style background music from a video plan.
//!
//! Pipeline of pure stages: parse -> validate -> plan (timing) -> arrangement
//! (note events by role) -> instrument resolution (allow-lists, fallbacks) ->
//! synthesis -> mix -> master -> encode.

pub mod harmony;
pub mod melody;
pub mod plan;
pub mod rng;
pub mod timing;
pub mod types;
