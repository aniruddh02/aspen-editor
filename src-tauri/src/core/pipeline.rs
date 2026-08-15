use image::GenericImageView;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

use crate::core::benchmark::{BenchmarkRecorder, WinnerDecision};
use crate::core::cache::{self, HashCache};
use crate::core::dedupe::{self, ImageRecord};
use crate::core::discover::{self, is_raw_ext};
use crate::core::fs_action;
use crate::core::preview;
use crate::core::quality::{self, ScoredMember};
use crate::core::settings::{AppSettings, GOOD_DIR, REJECTED_DIR};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub stage: String,
    pub message: String,
    pub current: u64,
    pub total: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeduplicateResult {
    pub folder: String,
    pub scanned: usize,
    pub duplicate_groups: usize,
    pub kept_good: usize,
    pub rejected: usize,
    pub unique_untouched: usize,
    pub errors: Vec<String>,
    pub good_dir: String,
    pub rejected_dir: String,
    pub ai_reranked: usize,
    /// Path of the burst-benchmark JSONL when diagnostic capture was enabled.
    pub benchmark_log: Option<String>,
}

fn file_name_str(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("image")
}

pub fn run_deduplicate<F>(
    root: &Path,
    settings: &AppSettings,
    cancel: Arc<AtomicBool>,
    mut on_progress: F,
) -> anyhow::Result<DeduplicateResult>
where
    F: FnMut(ProgressEvent),
{
    let log = |stage: &str, message: String, current: u64, total: u64, cb: &mut F| {
        cb(ProgressEvent {
            stage: stage.into(),
            message,
            current,
            total,
        });
    };

    if cancel.load(AtomicOrdering::Relaxed) {
        anyhow::bail!("cancelled");
    }

    log(
        "scan",
        format!("Scanning {}", root.display()),
        0,
        0,
        &mut on_progress,
    );

    let paths = discover::discover_images(
        root,
        settings.include_subfolders,
        &settings.enabled_extensions,
    );
    let total = paths.len() as u64;
    log(
        "scan",
        format!("Found {total} images"),
        total,
        total,
        &mut on_progress,
    );

    if paths.is_empty() {
        return Ok(DeduplicateResult {
            folder: root.display().to_string(),
            scanned: 0,
            duplicate_groups: 0,
            kept_good: 0,
            rejected: 0,
            unique_untouched: 0,
            errors: vec![],
            good_dir: root.join(GOOD_DIR).display().to_string(),
            rejected_dir: root.join(REJECTED_DIR).display().to_string(),
            ai_reranked: 0,
            benchmark_log: None,
        });
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(settings.perf_profile.max_threads())
        .build()?;

    let mut cache = HashCache::load();
    let max_edge = match settings.perf_profile {
        crate::core::settings::PerfProfile::Low => 512,
        crate::core::settings::PerfProfile::Medium => 768,
        crate::core::settings::PerfProfile::High => 1024,
    };

    log(
        "hash",
        "Computing hashes and previews…".into(),
        0,
        total,
        &mut on_progress,
    );

    let mut records: Vec<ImageRecord> = Vec::with_capacity(paths.len());
    let mut errors = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        if cancel.load(AtomicOrdering::Relaxed) {
            anyhow::bail!("cancelled");
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_raw = is_raw_ext(&ext);

        let blake3 = match cache::resolve_blake3(&mut cache, path) {
            Ok(h) => h,
            Err(e) => {
                errors.push(format!("{}: {e}", path.display()));
                continue;
            }
        };

        let (size, _) = cache::file_meta(path).unwrap_or((0, std::time::SystemTime::UNIX_EPOCH));

        let preview_result = pool.install(|| preview::load_preview(path, max_edge));
        let (phash, dhash, pw, ph) = match preview_result {
            Ok(img) => {
                let (w, h) = img.dimensions();
                let phv = dedupe::phash(&img);
                let dh = if settings.perf_profile.confirm_near_dupes() {
                    Some(dedupe::dhash(&img))
                } else {
                    None
                };
                (Some(phv), dh, w, h)
            }
            Err(e) => {
                errors.push(format!("preview {}: {e}", path.display()));
                (None, None, 0, 0)
            }
        };

        if let Ok((sz, mt)) = cache::file_meta(path) {
            let mtime_secs = mt
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            cache.insert(
                path,
                cache::CacheEntry {
                    size: sz,
                    mtime_secs,
                    blake3: blake3.clone(),
                    phash,
                    preview_w: Some(pw),
                    preview_h: Some(ph),
                },
            );
        }

        records.push(ImageRecord {
            path: path.clone(),
            blake3,
            phash,
            dhash,
            preview_w: pw,
            preview_h: ph,
            size,
            is_raw_or_dng: is_raw,
        });

        log(
            "hash",
            format!("Hashed {} / {} files", i + 1, paths.len()),
            (i + 1) as u64,
            total,
            &mut on_progress,
        );
    }

    cache
        .save()
        .map_err(|error| anyhow::anyhow!("ASPEN-CACHE-SAVE: {error}"))?;

    let threshold = settings.duplicate_strength.hamming_threshold();
    let confirm_dhash = settings.perf_profile.confirm_near_dupes();

    let mut benchmark = if settings.benchmark_logging {
        let run_id = uuid::Uuid::new_v4().to_string();
        BenchmarkRecorder::start(
            &run_id,
            root,
            settings,
            max_edge,
            threshold,
            confirm_dhash,
            records.len(),
        )
    } else {
        None
    };
    if let Some(recorder) = benchmark.as_ref() {
        log(
            "benchmark",
            format!("Benchmark log: {}", recorder.path().display()),
            0,
            total,
            &mut on_progress,
        );
    }

    let groups = dedupe::cluster_duplicates(&records, threshold, confirm_dhash);

    log(
        "cluster",
        format!("Found {} duplicate groups", groups.len()),
        groups.len() as u64,
        groups.len() as u64,
        &mut on_progress,
    );

    let in_group: std::collections::HashSet<usize> = groups.iter().flatten().copied().collect();
    let unique_indices: Vec<usize> = (0..records.len())
        .filter(|i| !in_group.contains(i))
        .collect();

    let (good_dir, rejected_dir) = fs_action::ensure_dest_dirs(root)?;

    let mut kept_good = 0usize;
    let mut rejected = 0usize;
    let mut ai_reranked = 0usize;
    let mut processed_files = 0usize;
    let total_to_process = records.len();
    // Resolve model capability once per run. Calling Ollama's /api/show for
    // every duplicate group adds seconds of avoidable latency.
    let ai_vision = settings.use_ai_for_dedup
        && settings.enable_ai_features
        && ollama_model_supports_vision(&settings.ollama_model);

    // --- Score and sort duplicate groups ---
    for (gi, group) in groups.iter().enumerate() {
        if cancel.load(AtomicOrdering::Relaxed) {
            anyhow::bail!("cancelled");
        }
        log(
            "score",
            format!(
                "Scoring group {} of {} ({} files)",
                gi + 1,
                groups.len(),
                group.len()
            ),
            processed_files as u64,
            total_to_process as u64,
            &mut on_progress,
        );

        let mut scored: Vec<ScoredMember> = Vec::new();
        let mut diagnostics: Vec<(usize, quality::QualityDiagnostics)> = Vec::new();
        for &idx in group {
            let path = &records[idx].path;
            match preview::load_preview(path, max_edge) {
                Ok(img) => {
                    let member = if benchmark.is_some() {
                        let (member, diag) = quality::score_member_detailed(
                            &img,
                            settings.scene_mode,
                            settings.perf_profile,
                        );
                        diagnostics.push((idx, diag));
                        member
                    } else {
                        quality::score_member(&img, settings.scene_mode, settings.perf_profile)
                    };
                    scored.push(member.with_index(idx));
                }
                Err(e) => {
                    errors.push(format!("score {}: {e}", path.display()));
                    scored.push(ScoredMember::zero(idx));
                }
            }
        }

        // Captured before normalization so absolute quality stays comparable
        // across groups when the log is analyzed later.
        let raw_scores: Vec<(usize, f64)> = scored.iter().map(|s| (s.index, s.score)).collect();

        quality::normalize_group_scores(&mut scored, group, settings.scene_mode);

        // Vision models inspect every group because semantic qualities such as
        // eyes-open and expression are not represented reliably by pixel
        // metrics. Text models are limited to close-score tie breaking.
        let algo_winner = quality::pick_winner(group, &records, &scored);
        let mut winner = algo_winner;
        let mut ai_choice: Option<usize> = None;
        if settings.use_ai_for_dedup && settings.enable_ai_features {
            if let Some(ai_winner) =
                try_ai_rerank(group, &records, &scored, winner, settings, ai_vision)
            {
                ai_choice = Some(ai_winner);
                if ai_winner != winner {
                    let old_name = records[winner]
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?");
                    let new_name = records[ai_winner]
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?");
                    log(
                        "ai-rerank",
                        format!("AI re-ranked group {gi}: {old_name} → {new_name}"),
                        processed_files as u64,
                        total_to_process as u64,
                        &mut on_progress,
                    );
                    winner = ai_winner;
                    ai_reranked += 1;
                }
            }
        }

        if let Some(ws) = scored.iter().find(|s| s.index == winner) {
            log(
                "score",
                format!(
                    "Winner {} (expr {:.1}, face {:.1}, sharp {:.1}, focus {:.2}, blur-conf {:.2}, exp {:.2}, vib {:.2}, score {:.3})",
                    file_name_str(&records[winner].path),
                    ws.expression,
                    ws.face,
                    ws.sharpness,
                    ws.focus,
                    ws.blur_confidence,
                    ws.exposure,
                    ws.vibrancy,
                    ws.score,
                ),
                processed_files as u64,
                total_to_process as u64,
                &mut on_progress,
            );
        }

        if let Some(recorder) = benchmark.as_mut() {
            for (idx, diag) in &diagnostics {
                if let Some(member) = scored.iter().find(|s| s.index == *idx) {
                    let raw = raw_scores
                        .iter()
                        .find(|(i, _)| i == idx)
                        .map(|(_, s)| *s)
                        .unwrap_or(member.score);
                    recorder.image(*idx, &records[*idx], member, raw, diag, Some(gi));
                }
            }
            recorder.group(
                gi,
                group,
                &records,
                &scored,
                WinnerDecision {
                    algo: algo_winner,
                    ai: ai_choice,
                    final_pick: winner,
                },
            );
        }

        for &idx in group {
            let path = &records[idx].path;
            let dest = if idx == winner {
                &good_dir
            } else {
                &rejected_dir
            };
            match fs_action::place_file(path, dest, settings.file_action) {
                Ok(dest_path) => {
                    processed_files += 1;
                    if let Some(recorder) = benchmark.as_mut() {
                        recorder.placement(
                            idx,
                            if idx == winner { "kept" } else { "rejected" },
                            file_name_str(&dest_path),
                        );
                    }
                    if idx == winner {
                        kept_good += 1;
                        log(
                            "move",
                            format!(
                                "Processed {processed_files}/{total_to_process} → Good {}",
                                file_name_str(&dest_path)
                            ),
                            processed_files as u64,
                            total_to_process as u64,
                            &mut on_progress,
                        );
                    } else {
                        rejected += 1;
                        log(
                            "move",
                            format!(
                                "Processed {processed_files}/{total_to_process} → Rejected {}",
                                file_name_str(&dest_path)
                            ),
                            processed_files as u64,
                            total_to_process as u64,
                            &mut on_progress,
                        );
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
    }

    // --- Move/copy unique (non-duplicate) files into Images-Good ---
    if !unique_indices.is_empty() {
        log(
            "unique",
            format!(
                "Placing {} unique images into Images-Good",
                unique_indices.len()
            ),
            processed_files as u64,
            total_to_process as u64,
            &mut on_progress,
        );
        for &idx in &unique_indices {
            if cancel.load(AtomicOrdering::Relaxed) {
                anyhow::bail!("cancelled");
            }
            let path = &records[idx].path;

            // Singletons are the negative examples of the benchmark: they show
            // whether the clustering threshold is splitting real bursts apart.
            if benchmark.is_some() {
                if let Ok(img) = preview::load_preview(path, max_edge) {
                    let (member, diag) = quality::score_member_detailed(
                        &img,
                        settings.scene_mode,
                        settings.perf_profile,
                    );
                    let member = member.with_index(idx);
                    if let Some(recorder) = benchmark.as_mut() {
                        recorder.image(idx, &records[idx], &member, member.score, &diag, None);
                    }
                }
            }

            match fs_action::place_file(path, &good_dir, settings.file_action) {
                Ok(dest_path) => {
                    processed_files += 1;
                    kept_good += 1;
                    if let Some(recorder) = benchmark.as_mut() {
                        recorder.placement(idx, "unique", file_name_str(&dest_path));
                    }
                    log(
                        "move",
                        format!(
                            "Processed {processed_files}/{total_to_process} → Good (unique) {}",
                            file_name_str(&dest_path)
                        ),
                        processed_files as u64,
                        total_to_process as u64,
                        &mut on_progress,
                    );
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
    }

    let benchmark_log = benchmark.as_mut().map(|recorder| {
        recorder.finish(records.len(), groups.len(), kept_good, rejected);
        recorder.path().display().to_string()
    });

    log(
        "done",
        format!(
            "Complete: {kept_good} best images in Images-Good, {rejected} rejected, {} unique",
            unique_indices.len()
        ),
        1,
        1,
        &mut on_progress,
    );

    if let Some(recorder) = benchmark.as_ref() {
        log(
            "benchmark",
            format!(
                "Benchmark data written to {} (label your keepers in {})",
                recorder.path().display(),
                file_name_str(recorder.labels_path())
            ),
            1,
            1,
            &mut on_progress,
        );
    }

    Ok(DeduplicateResult {
        folder: root.display().to_string(),
        scanned: records.len(),
        duplicate_groups: groups.len(),
        kept_good,
        rejected,
        unique_untouched: unique_indices.len(),
        errors,
        good_dir: good_dir.display().to_string(),
        rejected_dir: rejected_dir.display().to_string(),
        ai_reranked,
        benchmark_log,
    })
}

/// Detect if an Ollama model supports vision (image inputs).
/// Primary check: `/api/show` returns a `capabilities` array containing "vision".
/// Fallback: name-based heuristic for well-known vision model families.
pub fn ollama_model_supports_vision(model: &str) -> bool {
    if model.is_empty() {
        return false;
    }

    let api_check = || -> Option<bool> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .ok()?;
        let json: serde_json::Value = client
            .post("http://127.0.0.1:11434/api/show")
            .json(&serde_json::json!({ "name": model }))
            .send()
            .ok()?
            .json()
            .ok()?;
        let caps = json.get("capabilities")?.as_array()?;
        Some(caps.iter().any(|c| c.as_str() == Some("vision")))
    };

    if api_check() == Some(true) {
        return true;
    }

    model_name_looks_vision(model)
}

/// Heuristic: check if the model name matches known vision family patterns.
fn model_name_looks_vision(model: &str) -> bool {
    let m = model.to_lowercase();
    // Well-known vision-capable model families as of 2026
    const VISION_FAMILIES: &[&str] = &[
        "vl",
        "vision",
        "llava",
        "moondream",
        "minicpm-v",
        "bakllava",
        "gemma3",
        "pixtral",
        "internvl",
        "cogvlm",
    ];
    VISION_FAMILIES.iter().any(|family| m.contains(family))
}

/// Attempt LLM re-ranking when scores are close.
/// Uses vision (actual images) when the selected model supports it,
/// otherwise falls back to text-based scoring reasoning.
/// Returns Some(new_winner_index) if the AI picked a different winner.
fn try_ai_rerank(
    group: &[usize],
    records: &[ImageRecord],
    scores: &[ScoredMember],
    _algo_winner: usize,
    settings: &AppSettings,
    supports_vision: bool,
) -> Option<usize> {
    let members: Vec<&ScoredMember> = scores.iter().filter(|s| group.contains(&s.index)).collect();

    let max_score = members
        .iter()
        .map(|m| m.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let close_threshold = max_score * 0.15 + 1e-6;
    let close_count = members
        .iter()
        .filter(|m| (max_score - m.score) < close_threshold)
        .count();

    if supports_vision {
        tracing::info!(
            "AI re-rank using vision model {} for group of {}",
            settings.ollama_model,
            group.len()
        );
        rerank_with_vision(group, records, scores, settings)
            .or_else(|| rerank_with_scores(group, records, scores, settings))
    } else {
        if close_count < 2 {
            return None;
        }
        rerank_with_scores(group, records, scores, settings)
    }
}

/// Vision re-rank: encode small JPEG thumbnails of each candidate and ask
/// the model to look at them and pick based on eye-open state, expression,
/// and captured moment.
fn rerank_with_vision(
    group: &[usize],
    records: &[ImageRecord],
    scores: &[ScoredMember],
    settings: &AppSettings,
) -> Option<usize> {
    use base64::Engine;

    let mut candidate_images: Vec<String> = Vec::new();
    let mut candidate_map: Vec<(usize, String)> = Vec::new();

    // Vision models get ~384px thumbnails: big enough to see faces/eyes,
    // small enough to keep inference fast and payload sane.
    for &idx in group {
        let path = &records[idx].path;
        let Ok(img) = preview::load_preview(path, 384) else {
            continue;
        };
        let Ok(jpeg_bytes) = preview::to_jpeg_bytes(&img) else {
            continue;
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg_bytes);
        let fname = file_name_str(path).to_string();
        candidate_map.push((idx, fname));
        candidate_images.push(b64);
    }

    if candidate_images.len() < 2 {
        return None;
    }

    let mut prompt = String::from(
        "You are an expert photo curator comparing near-duplicate photos. \
The images are provided in the same order as the numbered list below.\n\n\
HARD RULES (in priority order):\n\
1. REJECT any photo where the main subject's EYES ARE CLOSED, mid-blink, or squinting.\n\
2. REJECT any photo where the main subject is BLURRED or out-of-focus.\n\
3. Among remaining photos, PREFER the one with the best EXPRESSION and captured MOMENT \
(natural smile, engaged gaze, peak action). A slightly softer photo with better expression \
BEATS a technically sharper but lifeless frame.\n\
4. If all photos are equally good, prefer the one with best exposure/color.\n\n\
Numbered candidates (matches image order):\n",
    );
    for (i, (idx, fname)) in candidate_map.iter().enumerate() {
        if let Some(s) = scores.iter().find(|s| s.index == *idx) {
            prompt.push_str(&format!(
                "{}. {fname} (algo scores: expression={:.2}, face={:.2}, exposure={:.2}, sharpness={:.2}, blur_conf={:.2})\n",
                i + 1,
                s.expression,
                s.face,
                s.exposure,
                s.sharpness,
                s.blur_confidence
            ));
        } else {
            prompt.push_str(&format!("{}. {fname}\n", i + 1));
        }
    }
    prompt.push_str(
        "\nRespond with ONLY the number of the best photo (e.g. \"2\") \
followed by a one-sentence reason. Example: \"2 — eyes fully open, natural smile\"",
    );

    // Vision inference is slower; allow a longer timeout.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .ok()?;

    let resp = client
        .post("http://127.0.0.1:11434/api/chat")
        .json(&serde_json::json!({
            "model": settings.ollama_model,
            "messages": [{
                "role": "user",
                "content": prompt,
                "images": candidate_images,
            }],
            "stream": false,
            "options": { "temperature": 0.1 },
        }))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .ok()?;

    let answer = resp
        .pointer("/message/content")
        .and_then(|v| v.as_str())?
        .trim()
        .to_string();

    tracing::info!("Vision re-rank answer: {}", answer);

    parse_vision_answer(&answer, &candidate_map)
}

/// Parse the model's response into a candidate index. Accepts either
/// a leading number ("2 — eyes open") or a filename mentioned in text.
fn parse_vision_answer(answer: &str, candidate_map: &[(usize, String)]) -> Option<usize> {
    // Strategy 1: leading number (1-indexed pointer into candidate_map)
    let leading_number: Option<usize> = answer
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok();

    if let Some(n) = leading_number {
        if n >= 1 && n <= candidate_map.len() {
            return Some(candidate_map[n - 1].0);
        }
    }

    // Strategy 2: filename substring match
    for (idx, fname) in candidate_map {
        if answer.contains(fname) {
            return Some(*idx);
        }
    }

    None
}

/// Text-only re-rank using the metric scores. Used when the selected
/// model does not support vision, or as a fallback if vision fails.
fn rerank_with_scores(
    group: &[usize],
    records: &[ImageRecord],
    scores: &[ScoredMember],
    settings: &AppSettings,
) -> Option<usize> {
    let mut prompt = String::from(
        "You are an expert photo curator selecting the best photo from near-duplicates. \
HARD RULES: \
1) REJECT any image where blur_confidence < 0.4 (severely out-of-focus or motion-blurred). \
2) REJECT any image where focus < 0.4 (subject is blurred). \
3) Among the remaining images, PRIORITIZE the moment/expression over sharpness — a slightly \
softer photo with a better expression or captured moment is preferred over a technically \
sharper but lifeless one. \
Metrics (all higher = better): expression (face energy/movement), face (face clarity), \
vibrancy (color richness), exposure (lighting), dynamic_range (tonal detail), sharpness, \
focus (subject in focus vs bg), blur_confidence (0 = blurred, 1 = sharp overall). \
Return ONLY the filename of the single best image, nothing else.\n\nCandidates:\n",
    );
    for &idx in group {
        let fname = file_name_str(&records[idx].path);
        if let Some(s) = scores.iter().find(|s| s.index == idx) {
            prompt.push_str(&format!(
                "- {fname}: expression={:.3}, face={:.3}, vibrancy={:.3}, exposure={:.3}, \
dynamic_range={:.3}, sharpness={:.3}, focus={:.3}, blur_confidence={:.3}, overall={:.3}\n",
                s.expression,
                s.face,
                s.vibrancy,
                s.exposure,
                s.dynamic_range,
                s.sharpness,
                s.focus,
                s.blur_confidence,
                s.score
            ));
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let resp = client
        .post("http://127.0.0.1:11434/api/generate")
        .json(&serde_json::json!({
            "model": settings.ollama_model,
            "prompt": prompt,
            "stream": false,
            "options": { "temperature": 0.1 },
        }))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .ok()?;

    let answer = resp.get("response")?.as_str()?.trim().to_string();

    for &idx in group {
        let fname = records[idx]
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if answer.contains(fname) || fname.contains(&answer) {
            return Some(idx);
        }
    }
    None
}

#[cfg(test)]
mod ai_rerank_tests {
    use super::*;

    #[test]
    fn vision_heuristic_recognizes_known_families() {
        assert!(model_name_looks_vision("qwen2.5vl:7b"));
        assert!(model_name_looks_vision("llama3.2-vision:11b"));
        assert!(model_name_looks_vision("llava:13b"));
        assert!(model_name_looks_vision("moondream:1.8b"));
        assert!(model_name_looks_vision("minicpm-v:8b"));
        assert!(model_name_looks_vision("gemma3:4b"));
        assert!(model_name_looks_vision("bakllava:7b"));
    }

    #[test]
    fn vision_heuristic_rejects_text_models() {
        assert!(!model_name_looks_vision("qwen3:1.7b"));
        assert!(!model_name_looks_vision("qwen3:8b"));
        assert!(!model_name_looks_vision("llama3.2:3b"));
        assert!(!model_name_looks_vision("llama3.3:70b"));
        assert!(!model_name_looks_vision("mistral:7b"));
        assert!(!model_name_looks_vision("phi3.5:3.8b"));
        assert!(!model_name_looks_vision(""));
    }

    #[test]
    fn parse_vision_answer_leading_number() {
        let map = vec![
            (10, "a.jpg".to_string()),
            (20, "b.jpg".to_string()),
            (30, "c.jpg".to_string()),
        ];
        assert_eq!(parse_vision_answer("2 — eyes open", &map), Some(20));
        assert_eq!(parse_vision_answer("1", &map), Some(10));
        assert_eq!(parse_vision_answer("3 - best expression", &map), Some(30));
    }

    #[test]
    fn parse_vision_answer_filename_fallback() {
        let map = vec![
            (10, "IMG_0001.jpg".to_string()),
            (20, "IMG_0002.jpg".to_string()),
        ];
        assert_eq!(
            parse_vision_answer("The best is IMG_0002.jpg", &map),
            Some(20)
        );
    }

    #[test]
    fn parse_vision_answer_out_of_range() {
        let map = vec![(10, "a.jpg".to_string())];
        assert_eq!(parse_vision_answer("5 — bad answer", &map), None);
        assert_eq!(parse_vision_answer("garbage response", &map), None);
    }
}
