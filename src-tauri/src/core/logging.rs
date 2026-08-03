use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_UI_EVENTS: usize = 2_000;
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_FILES: usize = 5;

static EVENTS: OnceLock<Mutex<VecDeque<LogEvent>>> = OnceLock::new();
static SENDER: OnceLock<mpsc::Sender<LogEvent>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub feature: String,
    pub route: String,
    pub stage: String,
    pub action: String,
    pub message: String,
    pub run_id: Option<String>,
    pub item_index: Option<usize>,
    pub item_total: Option<usize>,
    pub file_path: Option<String>,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub error_chain: Option<String>,
    pub metadata: BTreeMap<String, Value>,
}

impl LogEvent {
    pub fn new(
        level: LogLevel,
        feature: impl Into<String>,
        route: impl Into<String>,
        stage: impl Into<String>,
        action: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            level,
            feature: feature.into(),
            route: route.into(),
            stage: stage.into(),
            action: action.into(),
            message: message.into(),
            run_id: None,
            item_index: None,
            item_total: None,
            file_path: None,
            duration_ms: None,
            error_code: None,
            error_chain: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_error(mut self, code: impl Into<String>, chain: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self.error_chain = Some(chain.into());
        self
    }
}

pub fn init() {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_UI_EVENTS)));
    SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<LogEvent>();
        std::thread::Builder::new()
            .name("aspen-log-writer".into())
            .spawn(move || {
                while let Ok(event) = receiver.recv() {
                    if let Err(error) = write_event(&event) {
                        eprintln!("Aspen log write failed: {error}");
                    }
                }
            })
            .expect("failed to start Aspen log writer");
        sender
    });
}

pub fn record(event: LogEvent) {
    init();
    if let Some(events) = EVENTS.get() {
        if let Ok(mut events) = events.lock() {
            if events.len() == MAX_UI_EVENTS {
                events.pop_front();
            }
            events.push_back(event.clone());
        }
    }
    if let Some(sender) = SENDER.get() {
        if let Err(error) = sender.send(event) {
            eprintln!("Aspen log queue failed: {error}");
        }
    }
}

pub fn recent_events() -> Vec<LogEvent> {
    EVENTS
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .map(|events| events.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn clear() -> anyhow::Result<()> {
    if let Ok(mut events) = EVENTS.get_or_init(|| Mutex::new(VecDeque::new())).lock() {
        events.clear();
    }
    let directory = logs_dir()?;
    if directory.exists() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("aspen.log"))
                .unwrap_or(false)
            {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

pub fn logs_dir() -> anyhow::Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join("Library/Logs/Aspen"))
        .ok_or_else(|| anyhow::anyhow!("ASPEN-FS-LOGS: cannot resolve home directory"))
}

fn write_event(event: &LogEvent) -> anyhow::Result<()> {
    let directory = logs_dir()?;
    fs::create_dir_all(&directory)?;
    let path = directory.join("aspen.log");
    if path.metadata().map(|meta| meta.len()).unwrap_or(0) >= MAX_FILE_BYTES {
        rotate(&directory)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn rotate(directory: &std::path::Path) -> anyhow::Result<()> {
    let oldest = directory.join(format!("aspen.log.{}", MAX_FILES - 1));
    if oldest.exists() {
        fs::remove_file(oldest)?;
    }
    for index in (1..MAX_FILES - 1).rev() {
        let from = directory.join(format!("aspen.log.{index}"));
        let to = directory.join(format!("aspen.log.{}", index + 1));
        if from.exists() {
            fs::rename(from, to)?;
        }
    }
    let current = directory.join("aspen.log");
    if current.exists() {
        fs::rename(current, directory.join("aspen.log.1"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_contains_structured_route_fields() {
        let event = LogEvent::new(
            LogLevel::Info,
            "deduplicate",
            "run_deduplicate_cmd",
            "start",
            "run",
            "Started",
        )
        .with_run_id("run-1");
        assert_eq!(event.route, "run_deduplicate_cmd");
        assert_eq!(event.run_id.as_deref(), Some("run-1"));
    }

    #[test]
    fn rotation_keeps_bounded_number_of_files() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("aspen.log"), "current").unwrap();
        for index in 1..MAX_FILES {
            fs::write(
                directory.path().join(format!("aspen.log.{index}")),
                index.to_string(),
            )
            .unwrap();
        }
        rotate(directory.path()).unwrap();
        let files = fs::read_dir(directory.path()).unwrap().count();
        assert_eq!(files, MAX_FILES - 1);
        assert!(directory
            .path()
            .join(format!("aspen.log.{}", MAX_FILES - 1))
            .exists());
    }
}
