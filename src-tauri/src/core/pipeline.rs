use image::GenericImageView;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

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
    pub unique_left: usize,
    pub errors: Vec<String>,
    pub good_dir: String,
    pub rejected_dir: String,
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
            unique_left: 0,
            errors: vec![],
            good_dir: root.join(GOOD_DIR).display().to_string(),
            rejected_dir: root.join(REJECTED_DIR).display().to_string(),
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

    // Sequential cache updates with parallel preview where possible
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

        // Update cache phash
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

        if i % 5 == 0 || i + 1 == paths.len() {
            log(
                "hash",
                format!("Hashed {} / {}", i + 1, paths.len()),
                (i + 1) as u64,
                total,
                &mut on_progress,
            );
        }
    }

    let _ = cache.save();

    let threshold = settings.duplicate_strength.hamming_threshold();
    let groups = dedupe::cluster_duplicates(
        &records,
        threshold,
        settings.perf_profile.confirm_near_dupes(),
    );

    log(
        "cluster",
        format!("Found {} duplicate groups", groups.len()),
        groups.len() as u64,
        groups.len() as u64,
        &mut on_progress,
    );

    let in_group: std::collections::HashSet<usize> =
        groups.iter().flatten().copied().collect();
    let unique_left = records.len().saturating_sub(in_group.len());

    let (good_dir, rejected_dir) = fs_action::ensure_dest_dirs(root)?;

    let mut kept_good = 0usize;
    let mut rejected = 0usize;

    for (gi, group) in groups.iter().enumerate() {
        if cancel.load(AtomicOrdering::Relaxed) {
            anyhow::bail!("cancelled");
        }
        log(
            "score",
            format!("Scoring group {} ({} files)", gi + 1, group.len()),
            (gi + 1) as u64,
            groups.len() as u64,
            &mut on_progress,
        );

        let mut scored: Vec<ScoredMember> = Vec::new();
        for &idx in group {
            let path = &records[idx].path;
            match preview::load_preview(path, max_edge) {
                Ok(img) => {
                    let (raw_score, s, f) =
                        quality::score_member(&img, settings.scene_mode, settings.perf_profile);
                    scored.push(ScoredMember {
                        index: idx,
                        score: raw_score,
                        sharpness: s,
                        face: f,
                    });
                }
                Err(e) => {
                    errors.push(format!("score {}: {e}", path.display()));
                    scored.push(ScoredMember {
                        index: idx,
                        score: 0.0,
                        sharpness: 0.0,
                        face: 0.0,
                    });
                }
            }
        }

        quality::normalize_group_scores(&mut scored, group);
        let winner = quality::pick_winner(group, &records, &scored);

        for &idx in group {
            let path = &records[idx].path;
            let dest = if idx == winner {
                &good_dir
            } else {
                &rejected_dir
            };
            match fs_action::place_file(path, dest, settings.file_action) {
                Ok(dest_path) => {
                    if idx == winner {
                        kept_good += 1;
                        log(
                            "move",
                            format!("→ Good {}", dest_path.display()),
                            kept_good as u64,
                            group.len() as u64,
                            &mut on_progress,
                        );
                    } else {
                        rejected += 1;
                        log(
                            "move",
                            format!("→ Rejected {}", dest_path.display()),
                            rejected as u64,
                            group.len() as u64,
                            &mut on_progress,
                        );
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
    }

    log(
        "done",
        format!("Complete: {kept_good} kept, {rejected} rejected, {unique_left} unique"),
        1,
        1,
        &mut on_progress,
    );

    Ok(DeduplicateResult {
        folder: root.display().to_string(),
        scanned: records.len(),
        duplicate_groups: groups.len(),
        kept_good,
        rejected,
        unique_left,
        errors,
        good_dir: good_dir.display().to_string(),
        rejected_dir: rejected_dir.display().to_string(),
    })
}
