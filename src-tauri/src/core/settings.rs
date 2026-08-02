use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DOCS_URL: &str = "https://github.com/aniruddh02/aspen-editor#readme";
pub const GOOD_DIR: &str = "Images-Good";
pub const REJECTED_DIR: &str = "Rejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SceneMode {
    #[default]
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    #[default]
    Move,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PerfProfile {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DuplicateStrength {
    Loose,
    #[default]
    Balanced,
    Strict,
}

impl DuplicateStrength {
    /// Hamming distance threshold for pHash clustering.
    pub fn hamming_threshold(self) -> u32 {
        match self {
            Self::Loose => 12,
            Self::Balanced => 7,
            Self::Strict => 3,
        }
    }
}

impl PerfProfile {
    pub fn max_threads(self) -> usize {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        match self {
            Self::Low => (cpus / 4).max(1),
            Self::Medium => ((cpus * 3) / 4).max(2),
            Self::High => cpus.max(2),
        }
    }

    pub fn preview_batch(self) -> usize {
        match self {
            Self::Low => 2,
            Self::Medium => 4,
            Self::High => 8,
        }
    }

    pub fn face_scoring(self) -> bool {
        !matches!(self, Self::Low)
    }

    pub fn confirm_near_dupes(self) -> bool {
        matches!(self, Self::High)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub scene_mode: SceneMode,
    pub file_action: FileAction,
    pub perf_profile: PerfProfile,
    pub duplicate_strength: DuplicateStrength,
    pub include_subfolders: bool,
    pub enabled_extensions: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            scene_mode: SceneMode::Portrait,
            file_action: FileAction::Move,
            perf_profile: PerfProfile::Medium,
            duplicate_strength: DuplicateStrength::Balanced,
            include_subfolders: true,
            enabled_extensions: default_extensions(),
        }
    }
}

pub fn default_extensions() -> Vec<String> {
    [
        "arw", "srf", "sr2", "nef", "nrw", "cr2", "cr3", "crw", "raf", "dng", "jpg", "jpeg",
        "png", "tif", "tiff", "webp", "bmp", "gif", "heic", "heif",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn settings_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "aniruddh02", "Aspen")
        .map(|d| d.config_dir().join("settings.json"))
}

pub fn cache_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "aniruddh02", "Aspen")
        .map(|d| d.data_dir().join("hash-cache.json"))
}

pub fn load_settings() -> AppSettings {
    let Some(path) = settings_path() else {
        return AppSettings::default();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> anyhow::Result<()> {
    let Some(path) = settings_path() else {
        anyhow::bail!("could not resolve settings path");
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, json)?;
    Ok(())
}
