//! Aspen core — image discovery, deduplication, quality scoring, file actions.

pub mod cache;
pub mod dedupe;
pub mod discover;
pub mod fs_action;
pub mod pipeline;
#[cfg(test)]
mod pipeline_tests;
pub mod preview;
pub mod quality;
pub mod settings;
pub mod tags;

pub use pipeline::{run_deduplicate, DeduplicateResult, ProgressEvent};
pub use settings::{AppSettings, DuplicateStrength, FileAction, PerfProfile, SceneMode};
