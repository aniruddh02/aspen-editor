//! Burst-benchmark telemetry.
//!
//! Writes one JSONL file per deduplication run containing every raw metric,
//! intermediate value, and decision the scorer produced. The goal is that a
//! calibration run can be replayed and re-weighted from the log alone, so
//! thresholds can be tuned against real customer bursts without those photos
//! ever leaving the customer's machine.
//!
//! Privacy: image bytes are never written. Paths are recorded relative to the
//! scanned folder unless the user has opted into full paths for logging.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

use crate::core::dedupe::{self, ImageRecord};
use crate::core::quality::{QualityDiagnostics, ScoredMember};
use crate::core::settings::AppSettings;

/// Bump when the record shape changes so older logs stay interpretable.
pub const SCHEMA_VERSION: u32 = 1;

const MAX_RUN_FILES: usize = 20;

pub struct BenchmarkRecorder {
    file: File,
    path: PathBuf,
    labels_path: PathBuf,
    run_id: String,
    root: PathBuf,
    include_full_paths: bool,
    label_rows: Vec<LabelRow>,
}

/// Who chose the keeper for a group. Kept together so the three indices can't
/// be transposed at the call site.
#[derive(Debug, Clone, Copy)]
pub struct WinnerDecision {
    pub algo: usize,
    pub ai: Option<usize>,
    pub final_pick: usize,
}

/// One row of the labeling template the photographer fills in. Ground truth is
/// the whole point of the benchmark, and editing a spreadsheet is far more
/// realistic for them than hand-editing JSON.
struct LabelRow {
    group_index: usize,
    members: String,
    app_pick: String,
}

impl BenchmarkRecorder {
    /// Open a new benchmark log for this run. Returns `None` when the log
    /// directory cannot be created; benchmark capture must never fail a run.
    pub fn start(
        run_id: &str,
        root: &Path,
        settings: &AppSettings,
        max_edge: u32,
        hamming_threshold: u32,
        confirm_dhash: bool,
        total_images: usize,
    ) -> Option<Self> {
        Self::start_in(
            &benchmark_dir()?,
            run_id,
            root,
            settings,
            max_edge,
            hamming_threshold,
            confirm_dhash,
            total_images,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_in(
        directory: &Path,
        run_id: &str,
        root: &Path,
        settings: &AppSettings,
        max_edge: u32,
        hamming_threshold: u32,
        confirm_dhash: bool,
        total_images: usize,
    ) -> Option<Self> {
        if fs::create_dir_all(directory).is_err() {
            return None;
        }
        prune_old_runs(directory);

        let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let path = directory.join(format!("aspen-benchmark-{stamp}-{run_id}.jsonl"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;

        let mut recorder = Self {
            file,
            path,
            labels_path: directory.join(format!("aspen-labels-{stamp}-{run_id}.csv")),
            run_id: run_id.to_string(),
            root: root.to_path_buf(),
            include_full_paths: settings.include_full_paths_in_logs,
            label_rows: Vec::new(),
        };

        recorder.write(&RunRecord {
            record_type: "run",
            schema_version: SCHEMA_VERSION,
            run_id: run_id.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            folder: if recorder.include_full_paths {
                root.display().to_string()
            } else {
                // Folder name only: enough to tell shoots apart in a log
                // without disclosing the customer's directory layout.
                file_name(root)
            },
            total_images,
            max_edge,
            hamming_threshold,
            confirm_dhash,
            settings: settings_snapshot(settings),
        });

        Some(recorder)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn labels_path(&self) -> &Path {
        &self.labels_path
    }

    /// One record per image, whether it landed in a duplicate group or not.
    /// Singletons matter: they are the negative examples that tell us whether
    /// grouping thresholds are too tight.
    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &mut self,
        image_id: usize,
        record: &ImageRecord,
        member: &ScoredMember,
        raw_score: f64,
        diagnostics: &QualityDiagnostics,
        group_index: Option<usize>,
    ) {
        self.write(&ImageRecordEntry {
            record_type: "image",
            run_id: self.run_id.clone(),
            image_id,
            file_name: file_name(&record.path),
            path: self.folder_label(&record.path),
            extension: record
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase(),
            bytes: record.size,
            is_raw_or_dng: record.is_raw_or_dng,
            preview_w: record.preview_w,
            preview_h: record.preview_h,
            blake3_short: record.blake3.chars().take(16).collect(),
            phash: record.phash.map(|h| format!("{h:016x}")),
            dhash: record.dhash.map(|h| format!("{h:016x}")),
            group_index,
            metrics: Metrics {
                score: member.score,
                raw_score,
                sharpness: member.sharpness,
                focus: member.focus,
                face: member.face,
                exposure: member.exposure,
                expression: member.expression,
                vibrancy: member.vibrancy,
                dynamic_range: member.dynamic_range,
                blur_confidence: member.blur_confidence,
            },
            diagnostics: diagnostics.clone(),
        });
    }

    /// Group membership plus every pairwise hash distance. Without these the
    /// clustering threshold cannot be re-tuned from the log.
    pub fn group(
        &mut self,
        group_index: usize,
        group: &[usize],
        records: &[ImageRecord],
        scored: &[ScoredMember],
        decision: WinnerDecision,
    ) {
        let WinnerDecision {
            algo: algo_winner,
            ai: ai_winner,
            final_pick: final_winner,
        } = decision;

        let mut pairs = Vec::new();
        for (i, &a) in group.iter().enumerate() {
            for &b in group.iter().skip(i + 1) {
                pairs.push(PairDistance {
                    a,
                    b,
                    phash_distance: match (records[a].phash, records[b].phash) {
                        (Some(x), Some(y)) => Some(dedupe::hamming(x, y)),
                        _ => None,
                    },
                    dhash_distance: match (records[a].dhash, records[b].dhash) {
                        (Some(x), Some(y)) => Some(dedupe::hamming(x, y)),
                        _ => None,
                    },
                    identical_blake3: records[a].blake3 == records[b].blake3,
                });
            }
        }

        let mut ranking: Vec<RankedMember> = group
            .iter()
            .filter_map(|&idx| {
                scored
                    .iter()
                    .find(|s| s.index == idx)
                    .map(|s| RankedMember {
                        image_id: idx,
                        file_name: file_name(&records[idx].path),
                        normalized_score: s.score,
                    })
            })
            .collect();
        ranking.sort_by(|a, b| b.normalized_score.total_cmp(&a.normalized_score));

        self.label_rows.push(LabelRow {
            group_index,
            members: group
                .iter()
                .map(|&i| file_name(&records[i].path))
                .collect::<Vec<_>>()
                .join(" | "),
            app_pick: file_name(&records[final_winner].path),
        });

        let decision = if ai_winner.is_some_and(|w| w != algo_winner) {
            "ai-override"
        } else if ai_winner.is_some() {
            "ai-agreed"
        } else {
            "algorithm"
        };

        self.write(&GroupRecord {
            record_type: "group",
            run_id: self.run_id.clone(),
            group_index,
            size: group.len(),
            members: group.to_vec(),
            pairs,
            ranking,
            algo_winner,
            algo_winner_file: file_name(&records[algo_winner].path),
            ai_winner,
            ai_winner_file: ai_winner.map(|w| file_name(&records[w].path)),
            final_winner,
            final_winner_file: file_name(&records[final_winner].path),
            decision_path: decision,
            human_keeper: Value::Null,
        });
    }

    pub fn placement(&mut self, image_id: usize, outcome: &str, destination: &str) {
        self.write(&PlacementRecord {
            record_type: "placement",
            run_id: self.run_id.clone(),
            image_id,
            outcome: outcome.to_string(),
            destination: destination.to_string(),
        });
    }

    pub fn finish(&mut self, scanned: usize, groups: usize, kept: usize, rejected: usize) {
        self.write(&SummaryRecord {
            record_type: "summary",
            run_id: self.run_id.clone(),
            timestamp: Utc::now().to_rfc3339(),
            scanned,
            duplicate_groups: groups,
            kept_good: kept,
            rejected,
        });
        let _ = self.file.flush();
        self.write_label_template();
    }

    fn write_label_template(&self) {
        let mut csv = String::from(
            "group,files_in_group,aspen_picked,your_pick,your_reason,confidence_1_to_5\n",
        );
        for row in &self.label_rows {
            csv.push_str(&format!(
                "{},{},{},,,\n",
                row.group_index,
                csv_field(&row.members),
                csv_field(&row.app_pick),
            ));
        }
        let _ = fs::write(&self.labels_path, csv);
    }

    fn folder_label(&self, path: &Path) -> String {
        if self.include_full_paths {
            return path.display().to_string();
        }
        path.strip_prefix(&self.root)
            .unwrap_or_else(|_| Path::new(file_name_ref(path)))
            .display()
            .to_string()
    }

    fn write<T: Serialize>(&mut self, record: &T) {
        if let Ok(line) = serde_json::to_string(record) {
            let _ = writeln!(self.file, "{line}");
        }
    }
}

pub fn benchmark_dir() -> Option<PathBuf> {
    crate::core::logging::logs_dir()
        .ok()
        .map(|d| d.join("benchmark"))
}

/// Keep the newest runs only; a calibration session can produce many files.
fn prune_old_runs(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("aspen-benchmark-") && n.ends_with(".jsonl"))
        })
        .collect();
    if files.len() < MAX_RUN_FILES {
        return;
    }
    files.sort();
    let excess = files.len() + 1 - MAX_RUN_FILES;
    for path in files.into_iter().take(excess) {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let labels = name
                .replacen("aspen-benchmark-", "aspen-labels-", 1)
                .replacen(".jsonl", ".csv", 1);
            let _ = fs::remove_file(directory.join(labels));
        }
        let _ = fs::remove_file(path);
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn file_name(path: &Path) -> String {
    file_name_ref(path).to_string()
}

fn file_name_ref(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("image")
}

fn settings_snapshot(settings: &AppSettings) -> Value {
    serde_json::json!({
        "sceneMode": settings.scene_mode,
        "perfProfile": settings.perf_profile,
        "duplicateStrength": settings.duplicate_strength,
        "fileAction": settings.file_action,
        "includeSubfolders": settings.include_subfolders,
        "enableAiFeatures": settings.enable_ai_features,
        "useAiForDedup": settings.use_ai_for_dedup,
        "ollamaModel": settings.ollama_model,
        "ollamaTemperature": settings.ollama_temperature,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunRecord {
    #[serde(rename = "type")]
    record_type: &'static str,
    schema_version: u32,
    run_id: String,
    timestamp: String,
    app_version: String,
    os: String,
    arch: String,
    folder: String,
    total_images: usize,
    max_edge: u32,
    hamming_threshold: u32,
    confirm_dhash: bool,
    settings: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metrics {
    /// Score after in-group normalization; this is what ranked the burst.
    score: f64,
    /// Score before normalization, so absolute quality can be compared
    /// across groups and across runs.
    raw_score: f64,
    sharpness: f64,
    focus: f64,
    face: f64,
    exposure: f64,
    expression: f64,
    vibrancy: f64,
    dynamic_range: f64,
    blur_confidence: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageRecordEntry {
    #[serde(rename = "type")]
    record_type: &'static str,
    run_id: String,
    image_id: usize,
    file_name: String,
    path: String,
    extension: String,
    bytes: u64,
    is_raw_or_dng: bool,
    preview_w: u32,
    preview_h: u32,
    blake3_short: String,
    phash: Option<String>,
    dhash: Option<String>,
    group_index: Option<usize>,
    metrics: Metrics,
    diagnostics: QualityDiagnostics,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairDistance {
    a: usize,
    b: usize,
    phash_distance: Option<u32>,
    dhash_distance: Option<u32>,
    identical_blake3: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RankedMember {
    image_id: usize,
    file_name: String,
    normalized_score: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupRecord {
    #[serde(rename = "type")]
    record_type: &'static str,
    run_id: String,
    group_index: usize,
    size: usize,
    members: Vec<usize>,
    pairs: Vec<PairDistance>,
    ranking: Vec<RankedMember>,
    algo_winner: usize,
    algo_winner_file: String,
    ai_winner: Option<usize>,
    ai_winner_file: Option<String>,
    final_winner: usize,
    final_winner_file: String,
    decision_path: &'static str,
    /// Placeholder the photographer fills in when labeling the burst.
    human_keeper: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlacementRecord {
    #[serde(rename = "type")]
    record_type: &'static str,
    run_id: String,
    image_id: usize,
    outcome: String,
    destination: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryRecord {
    #[serde(rename = "type")]
    record_type: &'static str,
    run_id: String,
    timestamp: String,
    scanned: usize,
    duplicate_groups: usize,
    kept_good: usize,
    rejected: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_newest_runs() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..MAX_RUN_FILES + 3 {
            fs::write(
                dir.path().join(format!("aspen-benchmark-{i:04}-run.jsonl")),
                "{}",
            )
            .unwrap();
        }
        prune_old_runs(dir.path());
        let remaining = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(remaining, MAX_RUN_FILES - 1);
    }

    #[test]
    fn prune_ignores_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("aspen.log"), "x").unwrap();
        prune_old_runs(dir.path());
        assert!(dir.path().join("aspen.log").exists());
    }

    fn sample_record(root: &Path, name: &str) -> ImageRecord {
        ImageRecord {
            path: root.join(name),
            blake3: "abcdef0123456789".into(),
            phash: Some(0x00ff_00ff_00ff_00ff),
            dhash: Some(0x0f0f_0f0f_0f0f_0f0f),
            preview_w: 1024,
            preview_h: 683,
            size: 4096,
            is_raw_or_dng: false,
        }
    }

    fn read_records(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn run_log_captures_metrics_group_and_decision() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let settings = AppSettings::default();
        let records = vec![
            sample_record(root.path(), "burst-01.jpg"),
            sample_record(root.path(), "burst-02.jpg"),
        ];
        let scored = vec![
            ScoredMember {
                index: 0,
                score: 0.82,
                sharpness: 120.0,
                focus: 0.9,
                face: 300.0,
                exposure: 0.7,
                expression: 18.0,
                vibrancy: 0.4,
                dynamic_range: 0.6,
                blur_confidence: 1.0,
            },
            ScoredMember {
                index: 1,
                score: 0.41,
                ..ScoredMember::zero(1)
            },
        ];

        let mut recorder = BenchmarkRecorder::start_in(
            dir.path(),
            "run-1",
            root.path(),
            &settings,
            1024,
            6,
            true,
            2,
        )
        .unwrap();

        let diagnostics = QualityDiagnostics::default();
        recorder.image(0, &records[0], &scored[0], 0.77, &diagnostics, Some(0));
        recorder.image(1, &records[1], &scored[1], 0.39, &diagnostics, Some(0));
        recorder.group(
            0,
            &[0, 1],
            &records,
            &scored,
            WinnerDecision {
                algo: 0,
                ai: Some(1),
                final_pick: 1,
            },
        );
        recorder.placement(1, "kept", "burst-02.jpg");
        recorder.finish(2, 1, 1, 1);

        let entries = read_records(recorder.path());
        assert_eq!(entries[0]["type"], "run");
        assert_eq!(entries[0]["schemaVersion"], SCHEMA_VERSION);

        let image = &entries[1];
        assert_eq!(image["fileName"], "burst-01.jpg");
        // Relative path only: the customer's folder structure stays private.
        assert_eq!(image["path"], "burst-01.jpg");
        assert_eq!(image["metrics"]["rawScore"], 0.77);
        assert_eq!(image["metrics"]["sharpness"], 120.0);
        assert_eq!(image["phash"], "00ff00ff00ff00ff");

        let group = &entries[3];
        assert_eq!(group["type"], "group");
        assert_eq!(group["decisionPath"], "ai-override");
        assert_eq!(group["algoWinnerFile"], "burst-01.jpg");
        assert_eq!(group["finalWinnerFile"], "burst-02.jpg");
        assert_eq!(group["pairs"][0]["phashDistance"], 0);
        assert!(group["humanKeeper"].is_null());
        // Ranking is ordered so a labeler can see what the app preferred.
        assert_eq!(group["ranking"][0]["imageId"], 0);

        assert_eq!(entries[4]["type"], "placement");
        assert_eq!(entries[5]["type"], "summary");
        assert_eq!(entries[5]["keptGood"], 1);

        let labels = fs::read_to_string(recorder.labels_path()).unwrap();
        let rows: Vec<&str> = labels.lines().collect();
        assert!(rows[0].starts_with("group,files_in_group,aspen_picked,your_pick"));
        // The pipe-joined member list must be quoted-safe and the pick column
        // left blank for the photographer to complete.
        assert_eq!(rows[1], "0,burst-01.jpg | burst-02.jpg,burst-02.jpg,,,");
    }

    #[test]
    fn csv_fields_with_commas_are_quoted() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("plain.jpg"), "plain.jpg");
    }

    #[test]
    fn full_paths_are_only_logged_when_the_user_opts_in() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let settings = AppSettings {
            include_full_paths_in_logs: true,
            ..AppSettings::default()
        };
        let record = sample_record(root.path(), "burst-01.jpg");

        let mut recorder = BenchmarkRecorder::start_in(
            dir.path(),
            "run-2",
            root.path(),
            &settings,
            1024,
            6,
            true,
            1,
        )
        .unwrap();
        recorder.image(
            0,
            &record,
            &ScoredMember::zero(0),
            0.0,
            &QualityDiagnostics::default(),
            None,
        );

        let entries = read_records(recorder.path());
        assert_eq!(entries[1]["path"], record.path.display().to_string());
    }
}
