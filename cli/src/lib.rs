//! pixelreel: turns a storyboard, talking-head takes, ElevenLabs lines,
//! Pixel Lab scenes and outside animations into one video, scored with
//! cozy-score. See intents/features/INT-20261029-reel-pipeline/spec.md.

pub mod align;
pub mod assets;
pub mod audio;
pub mod captions;
pub mod config;
pub mod cuts;
pub mod files;
pub mod hash;
pub mod media;
pub mod pixellab;
pub mod proc;
pub mod project;
pub mod render;
pub mod score;
pub mod status;
pub mod storyboard;
pub mod takes;
pub mod time;
pub mod timeline;
pub mod vo;
