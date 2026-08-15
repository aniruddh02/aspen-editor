//! Integration-style pipeline tests with synthetic JPEGs.
use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use image::{DynamicImage, ImageBuffer, Rgb};

use crate::core::pipeline::{run_deduplicate, DeduplicateResult};
use crate::core::settings::{AppSettings, FileAction, SceneMode, GOOD_DIR, REJECTED_DIR};

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn run_pipeline(dir: &std::path::Path, settings: &AppSettings) -> DeduplicateResult {
    run_deduplicate(dir, settings, no_cancel(), |_| {}).unwrap()
}

fn write_jpeg(path: &std::path::Path, pattern: u8) {
    let img = DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |x, y| {
        let v = if ((x + y) as u8).wrapping_mul(pattern) % 2 == 0 {
            20u8
        } else {
            220u8
        };
        Rgb([v, v.wrapping_add(pattern), v])
    }));
    img.save(path).unwrap();
}

fn write_distinct_jpeg(path: &std::path::Path, base: u8) {
    let img = DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |x, y| {
        let v = ((x * 3 + y * 5) as u8).wrapping_add(base);
        Rgb([v, v.wrapping_mul(2), v.wrapping_add(50)])
    }));
    img.save(path).unwrap();
}

#[test]
fn pipeline_identical_jpegs_one_good_one_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jpg");
    let b = dir.path().join("b.jpg");
    write_jpeg(&a, 7);
    fs::copy(&a, &b).unwrap();

    let mut settings = AppSettings::default();
    settings.file_action = FileAction::Move;
    settings.scene_mode = SceneMode::Landscape;

    let result = run_pipeline(dir.path(), &settings);

    assert_eq!(result.duplicate_groups, 1);
    assert_eq!(result.kept_good, 1);
    assert_eq!(result.rejected, 1);
    assert!(dir.path().join(GOOD_DIR).read_dir().unwrap().count() == 1);
    assert!(dir.path().join(REJECTED_DIR).read_dir().unwrap().count() == 1);
}

#[test]
fn pipeline_unique_goes_to_good() {
    let dir = tempfile::tempdir().unwrap();
    write_jpeg(&dir.path().join("only.jpg"), 3);

    let settings = AppSettings::default();
    let result = run_pipeline(dir.path(), &settings);

    assert_eq!(result.duplicate_groups, 0);
    assert_eq!(result.unique_untouched, 1);
    assert_eq!(result.kept_good, 1, "unique files should be placed in Good");
    assert_eq!(dir.path().join(GOOD_DIR).read_dir().unwrap().count(), 1);
    // With Copy (default), original should still exist
    assert!(dir.path().join("only.jpg").exists());
}

#[test]
fn pipeline_copy_leaves_source() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jpg");
    let b = dir.path().join("b.jpg");
    write_jpeg(&a, 9);
    fs::copy(&a, &b).unwrap();

    let mut settings = AppSettings::default();
    settings.file_action = FileAction::Copy;

    let result = run_pipeline(dir.path(), &settings);

    assert!(a.exists(), "Copy mode should leave originals in place");
    assert!(b.exists());
    assert_eq!(dir.path().join(GOOD_DIR).read_dir().unwrap().count(), 1);
    assert_eq!(dir.path().join(REJECTED_DIR).read_dir().unwrap().count(), 1);
    assert_eq!(result.kept_good, 1);
    assert_eq!(result.rejected, 1);
}

#[test]
fn pipeline_mixed_dupes_and_uniques() {
    let dir = tempfile::tempdir().unwrap();
    // Two identical = 1 duplicate group
    let a = dir.path().join("dup1.jpg");
    let b = dir.path().join("dup2.jpg");
    write_jpeg(&a, 7);
    fs::copy(&a, &b).unwrap();
    // Two distinct unique images
    write_distinct_jpeg(&dir.path().join("unique1.jpg"), 100);
    write_distinct_jpeg(&dir.path().join("unique2.jpg"), 200);

    let mut settings = AppSettings::default();
    settings.file_action = FileAction::Copy;

    let result = run_pipeline(dir.path(), &settings);

    assert_eq!(result.scanned, 4);
    assert_eq!(result.duplicate_groups, 1);
    assert_eq!(result.rejected, 1);
    assert_eq!(result.unique_untouched, 2);
    // 1 winner from dup group + 2 uniques = 3 in Good
    assert_eq!(result.kept_good, 3);
    assert_eq!(dir.path().join(GOOD_DIR).read_dir().unwrap().count(), 3);
    assert_eq!(dir.path().join(REJECTED_DIR).read_dir().unwrap().count(), 1);
}

#[test]
fn pipeline_move_unique_removes_original() {
    let dir = tempfile::tempdir().unwrap();
    let only = dir.path().join("photo.jpg");
    write_jpeg(&only, 42);

    let mut settings = AppSettings::default();
    settings.file_action = FileAction::Move;

    let result = run_pipeline(dir.path(), &settings);

    assert_eq!(result.kept_good, 1);
    assert!(!only.exists(), "Move mode should remove the original");
    assert_eq!(dir.path().join(GOOD_DIR).read_dir().unwrap().count(), 1);
}

#[test]
fn sample_images_are_not_false_duplicates() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images");
    for strength in [
        crate::core::settings::DuplicateStrength::Loose,
        crate::core::settings::DuplicateStrength::Balanced,
        crate::core::settings::DuplicateStrength::Strict,
    ] {
        let dir = tempfile::tempdir().unwrap();
        for number in 1..=10 {
            fs::copy(
                source.join(format!("sample{number}.png")),
                dir.path().join(format!("sample{number}.png")),
            )
            .unwrap();
        }

        let mut settings = AppSettings::default();
        settings.file_action = FileAction::Copy;
        settings.scene_mode = SceneMode::Portrait;
        settings.perf_profile = crate::core::settings::PerfProfile::High;
        settings.duplicate_strength = strength;

        let result = run_pipeline(dir.path(), &settings);

        assert_eq!(result.scanned, 10);
        assert_eq!(
            result.duplicate_groups, 0,
            "{strength:?} must not cluster the ten distinct portraits"
        );
        assert_eq!(result.kept_good, 10);
        assert_eq!(result.rejected, 0);
    }
}

#[test]
fn sample_variant_group_keeps_the_sharp_well_exposed_original() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample8.png");
    let original = image::open(source).unwrap();
    let blurred = original.blur(7.0);
    let dark = original.brighten(-55);
    let dir = tempfile::tempdir().unwrap();

    original.save(dir.path().join("sharp.png")).unwrap();
    blurred.save(dir.path().join("blurred.png")).unwrap();
    dark.save(dir.path().join("dark.png")).unwrap();

    let settings = AppSettings {
        file_action: FileAction::Copy,
        scene_mode: SceneMode::Portrait,
        perf_profile: crate::core::settings::PerfProfile::High,
        duplicate_strength: crate::core::settings::DuplicateStrength::Strict,
        ..AppSettings::default()
    };
    let result = run_pipeline(dir.path(), &settings);

    assert_eq!(result.scanned, 3);
    assert_eq!(result.duplicate_groups, 1);
    // Strict complete-linkage keeps the dark frame unique (too far from the
    // sharp original at Hamming 3) instead of discarding it as a duplicate.
    assert_eq!(result.kept_good, 2);
    assert_eq!(result.rejected, 1);
    assert!(
        dir.path().join(GOOD_DIR).join("sharp.png").exists(),
        "the unmodified sharp, well-exposed portrait should be kept"
    );
    assert!(
        dir.path().join(REJECTED_DIR).join("blurred.png").exists(),
        "the blurred variant should still lose to the sharp original"
    );
}

#[test]
fn default_medium_balanced_still_groups_true_exposure_and_blur_variants() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample8.png");
    let original = image::open(source).unwrap();
    let dir = tempfile::tempdir().unwrap();

    original.save(dir.path().join("sharp.png")).unwrap();
    original
        .blur(7.0)
        .save(dir.path().join("blurred.png"))
        .unwrap();
    original
        .brighten(-55)
        .save(dir.path().join("dark.png"))
        .unwrap();

    let settings = AppSettings {
        file_action: FileAction::Copy,
        scene_mode: SceneMode::Portrait,
        perf_profile: crate::core::settings::PerfProfile::Medium,
        duplicate_strength: crate::core::settings::DuplicateStrength::Balanced,
        ..AppSettings::default()
    };
    let result = run_pipeline(dir.path(), &settings);

    assert_eq!(result.scanned, 3);
    assert_eq!(result.duplicate_groups, 1);
    assert_eq!(result.kept_good, 1);
    assert_eq!(result.rejected, 2);
    assert!(
        dir.path().join(GOOD_DIR).join("sharp.png").exists(),
        "the unmodified sharp, well-exposed portrait should win under default Medium/Balanced"
    );
}

/// End-to-end proof that a benchmark log carries enough to rebuild the burst
/// offline: group membership, hash distances, every metric, and the decision.
#[test]
fn benchmark_log_captures_a_full_burst_without_the_images() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample8.png");
    let original = image::open(source).unwrap();
    let dir = tempfile::tempdir().unwrap();
    original.save(dir.path().join("sharp.png")).unwrap();
    original
        .blur(7.0)
        .save(dir.path().join("blurred.png"))
        .unwrap();
    // A distinct portrait keeps at least one singleton in the run.
    image::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample1.png"),
    )
    .unwrap()
    .save(dir.path().join("other.png"))
    .unwrap();

    let settings = AppSettings {
        file_action: FileAction::Copy,
        scene_mode: SceneMode::Portrait,
        perf_profile: crate::core::settings::PerfProfile::High,
        duplicate_strength: crate::core::settings::DuplicateStrength::Strict,
        benchmark_logging: true,
        ..AppSettings::default()
    };
    let result = run_pipeline(dir.path(), &settings);

    let log_path = std::path::PathBuf::from(
        result
            .benchmark_log
            .expect("benchmark logging should report its output path"),
    );
    let contents = fs::read_to_string(&log_path).unwrap();
    let records: Vec<serde_json::Value> = contents
        .lines()
        .map(|line| serde_json::from_str(line).expect("every line must be valid JSON"))
        .collect();

    let by_type = |kind: &str| -> Vec<&serde_json::Value> {
        records.iter().filter(|r| r["type"] == kind).collect()
    };

    assert_eq!(by_type("run").len(), 1);
    assert_eq!(by_type("summary").len(), 1);
    // Grouped members and the singleton are all scored.
    assert_eq!(by_type("image").len(), 3);
    assert_eq!(by_type("group").len(), 1);

    let group = by_type("group")[0];
    assert_eq!(group["size"], 2);
    assert!(group["pairs"][0]["phashDistance"].is_number());
    assert_eq!(group["finalWinnerFile"], "sharp.png");

    let sharp = records
        .iter()
        .find(|r| r["type"] == "image" && r["fileName"] == "sharp.png")
        .unwrap();
    assert!(sharp["metrics"]["expression"].as_f64().unwrap() > 0.0);
    assert!(
        sharp["diagnostics"]["regionSharpness"]
            .as_array()
            .unwrap()
            .len()
            == 9
    );
    assert!(sharp["diagnostics"]["roiSkinFraction"].is_number());
    assert!(sharp["phash"].is_string());

    let singleton = records
        .iter()
        .find(|r| r["type"] == "image" && r["fileName"] == "other.png")
        .unwrap();
    assert!(singleton["groupIndex"].is_null());

    let labels = log_path.with_file_name(
        log_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replacen("aspen-benchmark-", "aspen-labels-", 1)
            .replacen(".jsonl", ".csv", 1),
    );
    let template = fs::read_to_string(&labels).unwrap();
    assert!(template.contains("your_pick"));
    assert!(template.contains("sharp.png"));

    if std::env::var("ASPEN_DUMP_BENCHMARK").is_ok() {
        eprintln!("--- jsonl bytes: {} ---", contents.len());
        for line in contents.lines() {
            eprintln!("{line}");
        }
        eprintln!("--- labels ---\n{template}");
    }

    let _ = fs::remove_file(&log_path);
    let _ = fs::remove_file(&labels);
}
