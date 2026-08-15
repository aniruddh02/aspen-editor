mod core;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use core::cache::HashCache;
use core::image_edit::{
    list_ollama_models, run_image_edit, send_chat, ImageEditProgress, ImageEditRequest,
    ImageEditResult,
};
use core::logging::{self, LogEvent, LogLevel};
use core::pipeline::{run_deduplicate, DeduplicateResult};
use core::settings::{load_settings, save_settings, AppSettings, DOCS_URL};

struct AppState {
    cancel: Mutex<Arc<AtomicBool>>,
}

#[tauri::command]
fn get_settings() -> AppSettings {
    logging::record(LogEvent::new(
        LogLevel::Info,
        "app",
        "get_settings",
        "load",
        "settings.load",
        "Loading settings",
    ));
    load_settings()
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> Result<(), String> {
    save_settings(&settings).map_err(|error| {
        let message = error.to_string();
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "app",
                "save_app_settings",
                "persist",
                "settings.save",
                "Settings save failed",
            )
            .with_error("ASPEN-FS-SETTINGS", &message),
        );
        message
    })?;
    logging::record(LogEvent::new(
        LogLevel::Info,
        "app",
        "save_app_settings",
        "persist",
        "settings.save",
        "Settings saved",
    ));
    Ok(())
}

#[tauri::command]
fn get_docs_url() -> String {
    logging::record(LogEvent::new(
        LogLevel::Info,
        "app",
        "get_docs_url",
        "help",
        "docs.resolve",
        "Documentation URL resolved",
    ));
    DOCS_URL.to_string()
}

#[tauri::command]
fn clear_hash_cache() -> Result<(), String> {
    HashCache::clear().map_err(|error| {
        let message = error.to_string();
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "deduplicate",
                "clear_hash_cache",
                "cache",
                "cache.clear",
                "Hash cache clear failed",
            )
            .with_error("ASPEN-CACHE-CLEAR", &message),
        );
        message
    })?;
    logging::record(LogEvent::new(
        LogLevel::Info,
        "deduplicate",
        "clear_hash_cache",
        "cache",
        "cache.clear",
        "Hash cache cleared",
    ));
    Ok(())
}

#[tauri::command]
fn cancel_deduplicate(state: State<'_, AppState>) {
    if let Ok(flag) = state.cancel.lock() {
        flag.store(true, Ordering::Relaxed);
    }
    logging::record(LogEvent::new(
        LogLevel::Info,
        "deduplicate",
        "cancel_deduplicate",
        "cancel",
        "run.cancel",
        "Cancellation requested",
    ));
}

#[tauri::command]
fn run_deduplicate_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    settings: AppSettings,
) -> Result<DeduplicateResult, String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    logging::record(
        LogEvent::new(
            LogLevel::Info,
            "deduplicate",
            "run_deduplicate_cmd",
            "start",
            "run.start",
            "Deduplicate started",
        )
        .with_run_id(&run_id),
    );
    if settings.verbose_logging {
        logging::record(
            LogEvent::new(
                LogLevel::Debug,
                "deduplicate",
                "run_deduplicate_cmd",
                "start",
                "settings.summary",
                format!(
                    "scene={:?}, strength={:?}, performance={:?}, action={:?}",
                    settings.scene_mode,
                    settings.duplicate_strength,
                    settings.perf_profile,
                    settings.file_action
                ),
            )
            .with_run_id(&run_id),
        );
    }
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut slot) = state.cancel.lock() {
        *slot = Arc::clone(&cancel);
    }

    let root = PathBuf::from(&folder);
    if !root.is_dir() {
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "deduplicate",
                "run_deduplicate_cmd",
                "validate",
                "folder.validate",
                "Selected path is not a directory",
            )
            .with_run_id(&run_id)
            .with_error("ASPEN-FS-SOURCE", format!("Not a directory: {folder}")),
        );
        return Err(format!("Not a directory: {folder}"));
    }

    save_settings(&settings).map_err(|error| error.to_string())?;

    let app_for_cb = app.clone();
    let callback_run_id = run_id.clone();
    let result = run_deduplicate(&root, &settings, cancel, move |ev| {
        if let Err(error) = app_for_cb.emit("dedupe-progress", &ev) {
            logging::record(
                LogEvent::new(
                    LogLevel::Warn,
                    "deduplicate",
                    "run_deduplicate_cmd",
                    &ev.stage,
                    "progress.emit",
                    "Progress event delivery failed",
                )
                .with_run_id(&callback_run_id)
                .with_error("ASPEN-DEDUPE-EMIT", error.to_string()),
            );
        }
    })
    .map_err(|error| {
        let message = error.to_string();
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "deduplicate",
                "run_deduplicate_cmd",
                "complete",
                "run.error",
                "Deduplicate failed",
            )
            .with_run_id(&run_id)
            .with_error("ASPEN-DEDUPE-RUN", &message),
        );
        message
    })?;

    let mut next_settings = settings;
    next_settings.last_images_good_path = result.good_dir.clone();
    save_settings(&next_settings).map_err(|error| error.to_string())?;
    logging::record(
        LogEvent::new(
            LogLevel::Info,
            "deduplicate",
            "run_deduplicate_cmd",
            "complete",
            "run.success",
            format!(
                "Deduplicate complete: {} groups, {} rejected",
                result.duplicate_groups, result.rejected
            ),
        )
        .with_run_id(run_id),
    );
    Ok(result)
}

#[tauri::command]
async fn run_image_edit_cmd(
    app: AppHandle,
    request: ImageEditRequest,
) -> Result<ImageEditResult, String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let verbose = request.settings.verbose_logging;
    logging::record(
        LogEvent::new(
            LogLevel::Info,
            "image-edit",
            "run_image_edit_cmd",
            "start",
            "run.start",
            "Image Editing started",
        )
        .with_run_id(&run_id),
    );
    if verbose {
        logging::record(
            LogEvent::new(
                LogLevel::Debug,
                "image-edit",
                "run_image_edit_cmd",
                "start",
                "settings.summary",
                format!(
                    "ai={}, sharpen={}({:?}), vignette={}({:?}), subjectBlur={}({:?})",
                    request.settings.use_ai_for_edit,
                    request.settings.eye_sharpen,
                    request.settings.eye_sharpen_strength,
                    request.settings.vignette,
                    request.settings.vignette_strength,
                    request.settings.subject_blur,
                    request.settings.subject_blur_strength,
                ),
            )
            .with_run_id(&run_id),
        );
    }
    let callback_run_id = run_id.clone();
    let result = run_image_edit(request, run_id.clone(), move |event: ImageEditProgress| {
        logging::record(
            LogEvent::new(
                if event.level == "warn" {
                    LogLevel::Warn
                } else {
                    LogLevel::Info
                },
                "image-edit",
                "run_image_edit_cmd",
                &event.stage,
                "progress",
                &event.message,
            )
            .with_run_id(&callback_run_id),
        );
        if let Err(error) = app.emit("image-edit-progress", event) {
            logging::record(
                LogEvent::new(
                    LogLevel::Warn,
                    "image-edit",
                    "run_image_edit_cmd",
                    "progress",
                    "progress.emit",
                    "Image Editing progress delivery failed",
                )
                .with_run_id(&callback_run_id)
                .with_error("ASPEN-EDIT-EMIT", error.to_string()),
            );
        }
    })
    .await
    .map_err(|error| {
        let message = format!("{error:#}");
        let code = message.split(':').next().unwrap_or("ASPEN-EDIT-RUN");
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "image-edit",
                "run_image_edit_cmd",
                "complete",
                "run.error",
                "Image Editing failed",
            )
            .with_run_id(&run_id)
            .with_error(code, &message),
        );
        message
    })?;
    logging::record(
        LogEvent::new(
            LogLevel::Info,
            "image-edit",
            "run_image_edit_cmd",
            "complete",
            "run.success",
            format!("Exported {} images", result.processed),
        )
        .with_run_id(run_id),
    );
    Ok(result)
}

#[tauri::command]
async fn list_ollama_models_cmd() -> Result<Vec<String>, String> {
    logging::record(LogEvent::new(
        LogLevel::Info,
        "image-edit",
        "list_ollama_models_cmd",
        "ollama",
        "models.list",
        "Discovering local Ollama models",
    ));
    list_ollama_models().await.map_err(|error| {
        let message = format!("{error:#}");
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "image-edit",
                "list_ollama_models_cmd",
                "ollama",
                "models.error",
                "Ollama model discovery failed",
            )
            .with_error("ASPEN-OLLAMA-MODELS", &message),
        );
        message
    })
}

#[tauri::command]
async fn send_ai_chat(
    model: String,
    temperature: f32,
    messages: Vec<serde_json::Value>,
) -> Result<String, String> {
    let message_count = messages.len();
    logging::record(LogEvent::new(
        LogLevel::Info,
        "image-edit",
        "send_ai_chat",
        "chat",
        "chat.send",
        format!("Sending AI chat context ({message_count} messages)"),
    ));
    send_chat(model, temperature, messages)
        .await
        .map_err(|error| {
            let message = format!("{error:#}");
            logging::record(
                LogEvent::new(
                    LogLevel::Error,
                    "image-edit",
                    "send_ai_chat",
                    "chat",
                    "chat.error",
                    "AI chat failed",
                )
                .with_error("ASPEN-OLLAMA-CHAT", &message),
            );
            message
        })
}

#[tauri::command]
fn record_ui_event(feature: String, action: String, message: String, level: String) {
    let level = match level.as_str() {
        "error" => LogLevel::Error,
        "warn" => LogLevel::Warn,
        "debug" => LogLevel::Debug,
        _ => LogLevel::Info,
    };
    logging::record(LogEvent::new(
        level,
        feature,
        "ui",
        "interaction",
        action,
        message,
    ));
}

#[tauri::command]
fn get_log_events() -> Vec<LogEvent> {
    logging::recent_events()
}

#[tauri::command]
fn get_logs_dir() -> Result<String, String> {
    logging::logs_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

/// Folder holding the per-run benchmark JSONL files. Created on demand so the
/// "reveal" action works even before the first diagnostic run.
#[tauri::command]
fn get_benchmark_dir() -> Result<String, String> {
    let dir = core::benchmark::benchmark_dir()
        .ok_or_else(|| "ASPEN-FS-LOGS: cannot resolve benchmark directory".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
fn clear_app_logs() -> Result<(), String> {
    logging::clear().map_err(|error| error.to_string())
}

#[tauri::command]
fn open_folder_path(path: String) -> Result<(), String> {
    let target = PathBuf::from(&path);
    core::paths::open_path(&target).map_err(|error| {
        let message = format!("{error:#}");
        logging::record(
            LogEvent::new(
                LogLevel::Error,
                "app",
                "open_folder_path",
                "open",
                "folder.open",
                "Failed to open folder",
            )
            .with_error("ASPEN-FS-OPEN", &message),
        );
        message
    })?;
    logging::record(LogEvent::new(
        LogLevel::Info,
        "app",
        "open_folder_path",
        "open",
        "folder.open",
        format!("Opened {path}"),
    ));
    Ok(())
}

#[tauri::command]
fn get_runtime_deps_status() -> core::paths::RuntimeDepsStatus {
    core::paths::runtime_deps_status()
}

#[tauri::command]
fn export_log(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    logging::record(LogEvent::new(
        LogLevel::Info,
        "app",
        "export_log",
        "export",
        "logs.export",
        "Log exported",
    ));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_app_settings,
            get_docs_url,
            clear_hash_cache,
            cancel_deduplicate,
            run_deduplicate_cmd,
            run_image_edit_cmd,
            list_ollama_models_cmd,
            send_ai_chat,
            record_ui_event,
            get_log_events,
            get_logs_dir,
            get_benchmark_dir,
            clear_app_logs,
            open_folder_path,
            get_runtime_deps_status,
            export_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Aspen");
}
