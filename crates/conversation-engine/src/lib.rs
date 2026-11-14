//! Offline CPU speech and native audio. No downloader or language-model authority.
mod config;
mod defaults;
pub mod device;
mod download;
mod model;
mod rng;
mod text;
mod tokenizer;
mod weights;
mod whisper;
pub mod worker;
