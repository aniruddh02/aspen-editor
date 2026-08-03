//! Integration-style pipeline tests with synthetic JPEGs.
use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use image::{DynamicImage, ImageBuffer, Rgb};

use crate::core::pipeline::run_deduplicate;
use crate::core::settings::{AppSettings, FileAction, SceneMode, GOOD_DIR, REJECTED_DIR};

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

    let result = run_deduplicate(
        dir.path(),
        &settings,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();

    assert_eq!(result.duplicate_groups, 1);
    assert_eq!(result.kept_good, 1);
    assert_eq!(result.rejected, 1);
    assert!(dir.path().join(GOOD_DIR).read_dir().unwrap().count() == 1);
    assert!(dir.path().join(REJECTED_DIR).read_dir().unwrap().count() == 1);
}

#[test]
fn pipeline_unique_untouched() {
    let dir = tempfile::tempdir().unwrap();
    write_jpeg(&dir.path().join("only.jpg"), 3);

    let settings = AppSettings::default();
    let result = run_deduplicate(
        dir.path(),
        &settings,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();

    assert_eq!(result.duplicate_groups, 0);
    assert_eq!(result.unique_left, 1);
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

    run_deduplicate(
        dir.path(),
        &settings,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();

    assert!(a.exists());
    assert!(b.exists());
    assert_eq!(dir.path().join(GOOD_DIR).read_dir().unwrap().count(), 1);
    assert_eq!(dir.path().join(REJECTED_DIR).read_dir().unwrap().count(), 1);
}
