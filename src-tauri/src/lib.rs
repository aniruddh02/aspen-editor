mod core;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use core::cache::HashCache;
use core::pipeline::{run_deduplicate, DeduplicateResult};
use core::settings::{load_settings, save_settings, AppSettings, DOCS_URL};

struct AppState {
    cancel: Mutex<Arc<AtomicBool>>,
}

#[tauri::command]
fn get_settings() -> AppSettings {
    load_settings()
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> Result<(), String> {
    save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_docs_url() -> String {
    DOCS_URL.to_string()
}

#[tauri::command]
fn clear_hash_cache() -> Result<(), String> {
    HashCache::clear().map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_deduplicate(state: State<'_, AppState>) {
    if let Ok(flag) = state.cancel.lock() {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn run_deduplicate_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: String,
    settings: AppSettings,
) -> Result<DeduplicateResult, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut slot) = state.cancel.lock() {
        *slot = Arc::clone(&cancel);
    }

    let root = PathBuf::from(&folder);
    if !root.is_dir() {
        return Err(format!("Not a directory: {folder}"));
    }

    let _ = save_settings(&settings);

    let app_for_cb = app.clone();
    run_deduplicate(&root, &settings, cancel, move |ev| {
        let _ = app_for_cb.emit("dedupe-progress", &ev);
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn export_log(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            export_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Aspen");
}
