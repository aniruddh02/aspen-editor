use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rmcp::{
    model::CallToolRequestParam,
    service::ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;

use super::settings::{AppSettings, EditStrength};

const OLLAMA_URL: &str = "http://127.0.0.1:11434";
const IMAGE_EXTENSIONS: &[&str] = &[
    "arw", "srf", "sr2", "nef", "nrw", "cr2", "cr3", "crw", "raf", "dng", "jpg", "jpeg", "png",
    "tif", "tiff", "webp", "bmp", "gif", "heic", "heif",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditRequest {
    pub source_path: String,
    pub settings: AppSettings,
    #[serde(default)]
    pub feedback: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditProgress {
    pub run_id: String,
    pub stage: String,
    pub message: String,
    pub current: usize,
    pub total: usize,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditResult {
    pub run_id: String,
    pub source_path: String,
    pub output_path: String,
    pub processed: usize,
    pub warnings: Vec<String>,
    pub used_ai: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    pub settings: BTreeMap<String, Value>,
    pub subject_blur_strength: Option<EditStrength>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AiAdjustments {
    eye_sharpen_strength: Option<EditStrength>,
    vignette_strength: Option<EditStrength>,
    subject_blur_strength: Option<EditStrength>,
    exposure_delta: Option<f64>,
    contrast_delta: Option<f64>,
    vibrance_delta: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

pub fn deterministic_plan(settings: &AppSettings) -> EditPlan {
    let mut values = BTreeMap::new();

    if settings.eye_sharpen {
        let (amount, radius, detail, masking) = sharpen_recipe(settings.eye_sharpen_strength);
        values.insert("Sharpness".into(), json!(amount));
        values.insert("SharpenRadius".into(), json!(radius));
        values.insert("SharpenDetail".into(), json!(detail));
        values.insert("SharpenEdgeMasking".into(), json!(masking));
    }
    if settings.vignette {
        let (amount, midpoint, roundness, feather) = vignette_recipe(settings.vignette_strength);
        values.insert("PostCropVignetteAmount".into(), json!(amount));
        values.insert("PostCropVignetteMidpoint".into(), json!(midpoint));
        values.insert("PostCropVignetteRoundness".into(), json!(roundness));
        values.insert("PostCropVignetteFeather".into(), json!(feather));
        values.insert("PostCropVignetteStyle".into(), json!(1));
    }
    if settings.white_balance {
        values.insert("WhiteBalance".into(), json!("Auto"));
    }
    if settings.color_tone {
        values.insert("Contrast2012".into(), json!(8));
        values.insert("Vibrance".into(), json!(8));
    }
    if settings.exposure_normalize {
        values.insert("Exposure2012".into(), json!(0.0));
        values.insert("Highlights2012".into(), json!(-10));
        values.insert("Shadows2012".into(), json!(10));
    }
    if settings.noise_reduction {
        values.insert("LuminanceSmoothing".into(), json!(20));
        values.insert("ColorNoiseReduction".into(), json!(25));
    }

    let mut warnings = Vec::new();
    if settings.optimal_crop {
        warnings.push(
            "Optimal crop requested; Lightroom MCP has no composition-analysis tool, so source crop is preserved."
                .into(),
        );
    }

    EditPlan {
        settings: values,
        subject_blur_strength: settings
            .subject_blur
            .then_some(settings.subject_blur_strength),
        warnings,
    }
}

pub async fn run_image_edit<F>(
    request: ImageEditRequest,
    run_id: String,
    mut progress: F,
) -> Result<ImageEditResult>
where
    F: FnMut(ImageEditProgress),
{
    let source = PathBuf::from(&request.source_path);
    if !source.is_dir() {
        return Err(anyhow!(
            "ASPEN-FS-SOURCE: not a directory: {}",
            source.display()
        ));
    }
    if request.settings.use_ai_for_edit && !request.settings.enable_ai_features {
        return Err(anyhow!(
            "ASPEN-OLLAMA-DISABLED: enable AI features before using AI"
        ));
    }

    let files = discover_images(&source)?;
    if files.is_empty() {
        return Err(anyhow!("ASPEN-EDITPLAN-EMPTY: no supported images found"));
    }
    let output = next_output_dir(&source)?;
    std::fs::create_dir_all(&output)
        .with_context(|| format!("ASPEN-FS-OUTPUT: cannot create {}", output.display()))?;

    emit(
        &mut progress,
        &run_id,
        "plan",
        "Building Lightroom edit plan",
        0,
        files.len(),
        "info",
    );
    let mut plan = deterministic_plan(&request.settings);
    if should_contact_ollama(&request.settings) {
        let adjustments = request_ai_adjustments(&request.settings, &request.feedback).await?;
        apply_ai_adjustments(&mut plan, adjustments);
    }

    let transport = TokioChildProcess::new(Command::new("npx").configure(|cmd| {
        cmd.args(["-y", "@mskalski/lightroom-mcp"]);
    }))
    .context("ASPEN-LRC-CONNECT-SPAWN: could not start npx")?;
    let service = tokio::time::timeout(Duration::from_secs(20), ().serve(transport))
        .await
        .context("ASPEN-LRC-CONNECT-TIMEOUT: Lightroom MCP initialization timed out")?
        .context(
            "ASPEN-LRC-CONNECT-INIT: start Lightroom and click Start Server in Plug-in Manager",
        )?;

    let tools = tokio::time::timeout(
        Duration::from_secs(20),
        service.list_tools(Default::default()),
    )
    .await
    .context("ASPEN-LRC-TOOL-TIMEOUT: capability discovery timed out")?
    .context("ASPEN-LRC-TOOL-LIST: capability discovery failed")?;
    let tool_names: Vec<&str> = tools.tools.iter().map(|tool| tool.name.as_ref()).collect();
    for required in ["import_photos", "set_develop_settings", "export_photos"] {
        if !tool_names.contains(&required) {
            if let Err(error) = service.cancel().await {
                tracing::warn!("Lightroom MCP cleanup failed: {error}");
            }
            return Err(anyhow!(
                "ASPEN-LRC-TOOL-MISSING: Lightroom MCP does not expose {required}"
            ));
        }
    }

    call_tool(
        &service,
        "import_photos",
        json!({ "source_path": source.to_string_lossy() }),
    )
    .await
    .context("ASPEN-LRC-TOOL-IMPORT: failed to import source folder")?;

    let mut warnings = plan.warnings.clone();
    if let Some(strength) = plan.subject_blur_strength {
        warnings.push(format!(
            "Subject blur ({strength:?}) skipped: Lightroom MCP 0.9 does not expose Lens Blur or subject masks."
        ));
    }

    for (index, file) in files.iter().enumerate() {
        emit(
            &mut progress,
            &run_id,
            "develop",
            &format!("Editing {}", file_name(file)),
            index + 1,
            files.len(),
            "info",
        );
        call_tool(
            &service,
            "set_develop_settings",
            json!({
                "photo_id": file.to_string_lossy(),
                "settings": plan.settings,
            }),
        )
        .await
        .with_context(|| format!("ASPEN-LRC-TOOL-DEVELOP: failed for {}", file.display()))?;
    }

    let photo_ids: Vec<String> = files
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    emit(
        &mut progress,
        &run_id,
        "export",
        "Exporting JPEG 90",
        files.len(),
        files.len(),
        "info",
    );
    call_tool(
        &service,
        "export_photos",
        json!({
            "photo_ids": photo_ids,
            "destination": output.to_string_lossy(),
            "format": "jpeg",
            "quality": 90,
        }),
    )
    .await
    .context("ASPEN-EXPORT-LRC: Lightroom export failed")?;
    service
        .cancel()
        .await
        .context("ASPEN-LRC-DISCONNECT: failed to stop MCP client")?;

    Ok(ImageEditResult {
        run_id,
        source_path: source.to_string_lossy().into_owned(),
        output_path: output.to_string_lossy().into_owned(),
        processed: files.len(),
        warnings,
        used_ai: request.settings.use_ai_for_edit,
    })
}

fn should_contact_ollama(settings: &AppSettings) -> bool {
    settings.enable_ai_features && settings.use_ai_for_edit
}

async fn call_tool(
    service: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &'static str,
    arguments: Value,
) -> Result<()> {
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        service.call_tool(CallToolRequestParam {
            name: name.into(),
            arguments: arguments.as_object().cloned(),
        }),
    )
    .await
    .with_context(|| format!("ASPEN-LRC-TOOL-TIMEOUT: {name} timed out"))??;
    if result.is_error == Some(true) {
        return Err(anyhow!(
            "{name} returned an MCP error: {:?}",
            result.content
        ));
    }
    Ok(())
}

async fn request_ai_adjustments(settings: &AppSettings, feedback: &str) -> Result<AiAdjustments> {
    let prompt = format!(
        "Return JSON only. Plan a tasteful portrait edit. Allowed keys: \
eyeSharpenStrength, vignetteStrength, subjectBlurStrength (small|medium|high), \
exposureDelta (-1..1), contrastDelta (-20..20), vibranceDelta (-20..20). \
User feedback: {feedback}"
    );
    let response = reqwest::Client::new()
        .post(format!("{OLLAMA_URL}/api/generate"))
        .json(&json!({
            "model": settings.ollama_model,
            "prompt": prompt,
            "stream": false,
            "format": "json",
            "options": { "temperature": settings.ollama_temperature },
        }))
        .send()
        .await
        .context("ASPEN-OLLAMA-CONNECT: Ollama is unavailable")?
        .error_for_status()
        .context("ASPEN-OLLAMA-HTTP: Ollama request failed")?
        .json::<OllamaGenerateResponse>()
        .await
        .context("ASPEN-OLLAMA-RESPONSE: invalid Ollama response")?;
    serde_json::from_str(&response.response)
        .context("ASPEN-EDITPLAN-SCHEMA: model returned invalid edit JSON")
}

fn apply_ai_adjustments(plan: &mut EditPlan, ai: AiAdjustments) {
    if let Some(strength) = ai.eye_sharpen_strength {
        let (amount, radius, detail, masking) = sharpen_recipe(strength);
        plan.settings.insert("Sharpness".into(), json!(amount));
        plan.settings.insert("SharpenRadius".into(), json!(radius));
        plan.settings.insert("SharpenDetail".into(), json!(detail));
        plan.settings
            .insert("SharpenEdgeMasking".into(), json!(masking));
    }
    if let Some(strength) = ai.vignette_strength {
        let (amount, midpoint, roundness, feather) = vignette_recipe(strength);
        plan.settings
            .insert("PostCropVignetteAmount".into(), json!(amount));
        plan.settings
            .insert("PostCropVignetteMidpoint".into(), json!(midpoint));
        plan.settings
            .insert("PostCropVignetteRoundness".into(), json!(roundness));
        plan.settings
            .insert("PostCropVignetteFeather".into(), json!(feather));
    }
    if let Some(strength) = ai.subject_blur_strength {
        plan.subject_blur_strength = Some(strength);
    }
    add_clamped(
        &mut plan.settings,
        "Exposure2012",
        ai.exposure_delta,
        -1.0,
        1.0,
    );
    add_clamped(
        &mut plan.settings,
        "Contrast2012",
        ai.contrast_delta,
        -20.0,
        20.0,
    );
    add_clamped(
        &mut plan.settings,
        "Vibrance",
        ai.vibrance_delta,
        -20.0,
        20.0,
    );
}

fn add_clamped(
    settings: &mut BTreeMap<String, Value>,
    key: &str,
    delta: Option<f64>,
    min: f64,
    max: f64,
) {
    let Some(delta) = delta else { return };
    let base = settings.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    settings.insert(key.into(), json!((base + delta).clamp(min, max)));
}

pub async fn list_ollama_models() -> Result<Vec<String>> {
    let response = reqwest::Client::new()
        .get(format!("{OLLAMA_URL}/api/tags"))
        .send()
        .await
        .context("ASPEN-OLLAMA-CONNECT: Ollama is unavailable")?
        .error_for_status()
        .context("ASPEN-OLLAMA-HTTP: model list failed")?
        .json::<OllamaTagsResponse>()
        .await
        .context("ASPEN-OLLAMA-RESPONSE: invalid model list")?;
    Ok(response
        .models
        .into_iter()
        .map(|model| model.name)
        .collect())
}

pub async fn send_chat(model: String, temperature: f32, messages: Vec<Value>) -> Result<String> {
    let trimmed = trim_chat_messages(messages);
    let response = reqwest::Client::new()
        .post(format!("{OLLAMA_URL}/api/chat"))
        .json(&json!({
            "model": model,
            "messages": trimmed,
            "stream": false,
            "options": { "temperature": temperature },
        }))
        .send()
        .await
        .context("ASPEN-OLLAMA-CONNECT: Ollama is unavailable")?
        .error_for_status()
        .context("ASPEN-OLLAMA-HTTP: chat failed")?
        .json::<Value>()
        .await
        .context("ASPEN-OLLAMA-RESPONSE: invalid chat response")?;
    response
        .pointer("/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("ASPEN-OLLAMA-RESPONSE: missing assistant message"))
}

fn trim_chat_messages(messages: Vec<Value>) -> Vec<Value> {
    const MAX_MESSAGES: usize = 20;
    const MAX_APPROX_CHARS: usize = 32_000;
    let mut kept = Vec::new();
    let mut chars = 0;
    for message in messages.into_iter().rev().take(MAX_MESSAGES) {
        let length = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::len)
            .unwrap_or(0);
        if !kept.is_empty() && chars + length > MAX_APPROX_CHARS {
            break;
        }
        chars += length;
        kept.push(message);
    }
    kept.reverse();
    kept
}

fn sharpen_recipe(strength: EditStrength) -> (i32, f32, i32, i32) {
    match strength {
        EditStrength::Small => (25, 1.0, 25, 70),
        EditStrength::Medium => (40, 1.0, 35, 80),
        EditStrength::High => (55, 1.2, 40, 85),
    }
}

fn vignette_recipe(strength: EditStrength) -> (i32, i32, i32, i32) {
    match strength {
        EditStrength::Small => (-10, 50, 0, 50),
        EditStrength::Medium => (-20, 45, 0, 60),
        EditStrength::High => (-35, 40, 0, 70),
    }
}

fn discover_images(source: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(source)? {
        let path = entry?.path();
        let supported = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
        if path.is_file() && supported {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub fn next_output_dir(source: &Path) -> Result<PathBuf> {
    let parent = source
        .parent()
        .ok_or_else(|| anyhow!("ASPEN-FS-OUTPUT: source has no parent"))?;
    let first = parent.join("Processed-Images");
    if !first.exists() {
        return Ok(first);
    }
    for version in 2..10_000 {
        let candidate = parent.join(format!("Processed-Images-{version}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(anyhow!("ASPEN-FS-OUTPUT: no output version available"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string()
}

fn emit<F>(
    callback: &mut F,
    run_id: &str,
    stage: &str,
    message: &str,
    current: usize,
    total: usize,
    level: &str,
) where
    F: FnMut(ImageEditProgress),
{
    callback(ImageEditProgress {
        run_id: run_id.into(),
        stage: stage.into(),
        message: message.into(),
        current,
        total,
        level: level.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strengths_produce_distinct_recipes() {
        assert_ne!(
            sharpen_recipe(EditStrength::Small),
            sharpen_recipe(EditStrength::Medium)
        );
        assert_ne!(
            sharpen_recipe(EditStrength::Medium),
            sharpen_recipe(EditStrength::High)
        );
        assert_ne!(
            vignette_recipe(EditStrength::Small),
            vignette_recipe(EditStrength::Medium)
        );
        assert_ne!(
            vignette_recipe(EditStrength::Medium),
            vignette_recipe(EditStrength::High)
        );
    }

    #[test]
    fn deterministic_plan_uses_defaults_without_ai() {
        let settings = AppSettings::default();
        let plan = deterministic_plan(&settings);
        assert_eq!(plan.settings["Sharpness"], json!(40));
        assert_eq!(plan.settings["PostCropVignetteAmount"], json!(-20));
        assert_eq!(plan.subject_blur_strength, Some(EditStrength::Medium));
    }

    #[test]
    fn ai_off_path_never_selects_ollama() {
        let mut settings = AppSettings::default();
        assert!(!should_contact_ollama(&settings));
        settings.enable_ai_features = true;
        assert!(!should_contact_ollama(&settings));
        settings.use_ai_for_edit = true;
        assert!(should_contact_ollama(&settings));
    }

    #[test]
    fn disabled_recipes_are_absent_from_plan() {
        let mut settings = AppSettings::default();
        settings.eye_sharpen = false;
        settings.vignette = false;
        settings.subject_blur = false;
        let plan = deterministic_plan(&settings);
        assert!(!plan.settings.contains_key("Sharpness"));
        assert!(!plan.settings.contains_key("PostCropVignetteAmount"));
        assert_eq!(plan.subject_blur_strength, None);
    }

    #[test]
    fn output_folder_versions_without_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("Images-Good");
        std::fs::create_dir(&source).unwrap();
        assert_eq!(
            next_output_dir(&source).unwrap(),
            temp.path().join("Processed-Images")
        );
        std::fs::create_dir(temp.path().join("Processed-Images")).unwrap();
        assert_eq!(
            next_output_dir(&source).unwrap(),
            temp.path().join("Processed-Images-2")
        );
    }

    #[test]
    fn chat_context_is_bounded_by_messages_and_approximate_tokens() {
        let messages = (0..30)
            .map(|index| json!({ "role": "user", "content": format!("{index}-{}", "x".repeat(2_000)) }))
            .collect();
        let trimmed = trim_chat_messages(messages);
        assert!(trimmed.len() <= 20);
        let chars: usize = trimmed
            .iter()
            .filter_map(|message| message["content"].as_str())
            .map(str::len)
            .sum();
        assert!(chars <= 32_000);
    }
}
