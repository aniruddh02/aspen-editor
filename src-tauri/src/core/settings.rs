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
    Move,
    #[default]
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

    #[allow(dead_code)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EditStrength {
    Small,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub scene_mode: SceneMode,
    pub file_action: FileAction,
    pub perf_profile: PerfProfile,
    pub duplicate_strength: DuplicateStrength,
    pub include_subfolders: bool,
    pub enabled_extensions: Vec<String>,
    pub continue_to_image_editing: bool,
    pub last_images_good_path: String,
    pub enable_ai_features: bool,
    pub use_ai_for_dedup: bool,
    pub use_ai_for_edit: bool,
    pub eye_sharpen: bool,
    pub eye_sharpen_strength: EditStrength,
    pub vignette: bool,
    pub vignette_strength: EditStrength,
    pub subject_blur: bool,
    pub subject_blur_strength: EditStrength,
    pub optimal_crop: bool,
    pub white_balance: bool,
    pub color_tone: bool,
    pub exposure_normalize: bool,
    pub noise_reduction: bool,
    pub ollama_model: String,
    pub ollama_temperature: f32,
    pub chat_auto_clear_after_run: bool,
    pub chat_auto_clear_on_leave: bool,
    pub chat_auto_clear_on_ai_off: bool,
    pub verbose_logging: bool,
    pub include_full_paths_in_logs: bool,
    pub include_chat_prompts_in_logs: bool,
    /// Write a per-run JSONL of every quality metric and ranking decision.
    /// Off by default: it adds a scoring pass over non-duplicate images.
    pub benchmark_logging: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            scene_mode: SceneMode::Portrait,
            file_action: FileAction::Copy,
            perf_profile: PerfProfile::Medium,
            duplicate_strength: DuplicateStrength::Balanced,
            include_subfolders: true,
            enabled_extensions: default_extensions(),
            continue_to_image_editing: true,
            last_images_good_path: String::new(),
            enable_ai_features: false,
            use_ai_for_dedup: false,
            use_ai_for_edit: false,
            eye_sharpen: true,
            eye_sharpen_strength: EditStrength::Medium,
            vignette: true,
            vignette_strength: EditStrength::Medium,
            subject_blur: true,
            subject_blur_strength: EditStrength::Medium,
            optimal_crop: true,
            white_balance: true,
            color_tone: true,
            exposure_normalize: true,
            noise_reduction: false,
            ollama_model: String::new(),
            ollama_temperature: 0.2,
            chat_auto_clear_after_run: true,
            chat_auto_clear_on_leave: true,
            chat_auto_clear_on_ai_off: true,
            verbose_logging: false,
            include_full_paths_in_logs: false,
            include_chat_prompts_in_logs: false,
            benchmark_logging: false,
        }
    }
}

pub fn default_extensions() -> Vec<String> {
    [
        "arw", "srf", "sr2", "nef", "nrw", "cr2", "cr3", "crw", "raf", "dng", "jpg", "jpeg", "png",
        "tif", "tiff", "webp", "bmp", "gif", "heic", "heif",
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
        crate::core::logging::record(
            crate::core::logging::LogEvent::new(
                crate::core::logging::LogLevel::Warn,
                "app",
                "get_settings",
                "load",
                "settings.default",
                "Settings path unavailable; using defaults",
            )
            .with_error("ASPEN-FS-SETTINGS-PATH", "Could not resolve settings path"),
        );
        return AppSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(settings) => settings,
            Err(error) => {
                crate::core::logging::record(
                    crate::core::logging::LogEvent::new(
                        crate::core::logging::LogLevel::Warn,
                        "app",
                        "get_settings",
                        "load",
                        "settings.default",
                        "Settings file is invalid; using defaults",
                    )
                    .with_error("ASPEN-FS-SETTINGS-PARSE", error.to_string()),
                );
                AppSettings::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
        Err(error) => {
            crate::core::logging::record(
                crate::core::logging::LogEvent::new(
                    crate::core::logging::LogLevel::Warn,
                    "app",
                    "get_settings",
                    "load",
                    "settings.default",
                    "Settings read failed; using defaults",
                )
                .with_error("ASPEN-FS-SETTINGS-READ", error.to_string()),
            );
            AppSettings::default()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_defaults_preserve_bulk_first_workflow() {
        let settings = AppSettings::default();
        assert!(settings.continue_to_image_editing);
        assert!(!settings.enable_ai_features);
        assert!(!settings.use_ai_for_dedup);
        assert!(!settings.use_ai_for_edit);
        assert!(settings.eye_sharpen);
        assert_eq!(settings.eye_sharpen_strength, EditStrength::Medium);
        assert!(settings.vignette);
        assert!(settings.subject_blur);
        assert!(!settings.noise_reduction);
        assert_eq!(settings.ollama_model, "");
    }

    #[test]
    fn v1_settings_json_migrates_with_v2_defaults() {
        let old = r#"{
            "sceneMode":"portrait",
            "fileAction":"move",
            "perfProfile":"medium",
            "duplicateStrength":"balanced",
            "includeSubfolders":true,
            "enabledExtensions":["jpg"]
        }"#;
        let settings: AppSettings = serde_json::from_str(old).unwrap();
        assert!(settings.continue_to_image_editing);
        assert!(settings.eye_sharpen);
        assert!(!settings.enable_ai_features);
        assert_eq!(settings.enabled_extensions, vec!["jpg"]);
    }
}
