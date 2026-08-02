use std::fs;
use std::path::{Path, PathBuf};

use crate::core::settings::{FileAction, GOOD_DIR, REJECTED_DIR};

#[allow(dead_code)]
pub struct ActionResult {
    pub good_count: usize,
    pub rejected_count: usize,
    pub errors: Vec<String>,
}

/// Ensure destination folders exist and apply macOS Finder label colors.
pub fn ensure_dest_dirs(root: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let good = root.join(GOOD_DIR);
    let rejected = root.join(REJECTED_DIR);
    fs::create_dir_all(&good)?;
    fs::create_dir_all(&rejected)?;
    crate::core::tags::apply_folder_tag(&good, crate::core::tags::FolderColor::Green);
    crate::core::tags::apply_folder_tag(&rejected, crate::core::tags::FolderColor::Red);
    Ok((good, rejected))
}

pub fn place_file(
    src: &Path,
    dest_dir: &Path,
    action: FileAction,
) -> anyhow::Result<PathBuf> {
    let name = unique_name(dest_dir, src.file_name().unwrap_or_default())?;
    let dest = dest_dir.join(&name);
    match action {
        FileAction::Move => {
            if fs::rename(src, &dest).is_err() {
                fs::copy(src, &dest)?;
                fs::remove_file(src)?;
            }
        }
        FileAction::Copy => {
            fs::copy(src, &dest)?;
        }
    }
    Ok(dest)
}

fn unique_name(dest_dir: &Path, file_name: &std::ffi::OsStr) -> anyhow::Result<std::ffi::OsString> {
    let original = PathBuf::from(file_name);
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let mut candidate = original.as_os_str().to_os_string();
    let mut n = 2;
    while dest_dir.join(&candidate).exists() {
        candidate = std::ffi::OsString::from(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn move_and_copy() {
        let dir = tempdir().unwrap();
        let (good, rejected) = ensure_dest_dirs(dir.path()).unwrap();

        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        fs::write(&a, b"aaa").unwrap();
        fs::write(&b, b"bbb").unwrap();

        place_file(&a, &good, FileAction::Move).unwrap();
        assert!(!a.exists());
        assert!(good.join("a.jpg").exists());

        place_file(&b, &rejected, FileAction::Copy).unwrap();
        assert!(b.exists());
        assert!(rejected.join("b.jpg").exists());
    }

    #[test]
    fn collision_suffix() {
        let dir = tempdir().unwrap();
        let (good, _) = ensure_dest_dirs(dir.path()).unwrap();
        fs::write(good.join("x.jpg"), b"1").unwrap();
        let src = dir.path().join("x.jpg");
        fs::write(&src, b"2").unwrap();
        let dest = place_file(&src, &good, FileAction::Copy).unwrap();
        assert!(dest.file_name().unwrap().to_string_lossy().contains("(2)"));
    }
}
