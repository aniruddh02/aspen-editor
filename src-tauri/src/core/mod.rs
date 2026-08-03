//! Aspen core — image discovery, deduplication, quality scoring, file actions.

pub mod cache;
pub mod dedupe;
pub mod discover;
pub mod fs_action;
pub mod image_edit;
pub mod logging;
pub mod pipeline;
#[cfg(test)]
mod pipeline_tests;
pub mod preview;
pub mod quality;
pub mod settings;
pub mod tags;

#[allow(unused_imports)]
pub use pipeline::{DeduplicateResult, ProgressEvent};
// Re-exports kept for crate consumers / docs
#[allow(unused_imports)]
pub use pipeline::run_deduplicate;
#[allow(unused_imports)]
pub use settings::{
    AppSettings, DuplicateStrength, EditStrength, FileAction, PerfProfile, SceneMode,
};
