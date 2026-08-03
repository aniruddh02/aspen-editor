use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::core::settings::{GOOD_DIR, REJECTED_DIR};

/// Camera RAW extensions.
pub fn is_raw_ext(ext: &str) -> bool {
    matches!(
        ext,
        "arw" | "srf" | "sr2" | "nef" | "nrw" | "cr2" | "cr3" | "crw" | "raf" | "dng"
    )
}

pub fn is_supported_ext(ext: &str, enabled: &[String]) -> bool {
    let e = ext.to_ascii_lowercase();
    enabled.iter().any(|x| x.eq_ignore_ascii_case(&e))
}

/// Recursively discover image files, skipping Images-Good and Rejected.
pub fn discover_images(
    root: &Path,
    include_subfolders: bool,
    enabled_extensions: &[String],
) -> Vec<PathBuf> {
    let walker = if include_subfolders {
        WalkDir::new(root).into_iter()
    } else {
        WalkDir::new(root).max_depth(1).into_iter()
    };

    let mut out = Vec::new();
    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if should_skip_path(path, root) {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if is_supported_ext(&ext, enabled_extensions) {
            out.push(path.to_path_buf());
        }
    }
    out.sort();
    out
}

fn should_skip_path(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == GOOD_DIR || s == REJECTED_DIR
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::settings::default_extensions;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_jpeg_and_skips_good_folder() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        fs::write(dir.path().join("b.png"), b"x").unwrap();
        fs::write(dir.path().join("skip.txt"), b"x").unwrap();
        let good = dir.path().join(GOOD_DIR);
        fs::create_dir(&good).unwrap();
        fs::write(good.join("c.jpg"), b"x").unwrap();

        let found = discover_images(dir.path(), true, &default_extensions());
        assert_eq!(found.len(), 2);
        assert!(found
            .iter()
            .all(|p| !p.to_string_lossy().contains(GOOD_DIR)));
    }

    #[test]
    fn raw_ext_detection() {
        assert!(is_raw_ext("arw"));
        assert!(is_raw_ext("dng"));
        assert!(!is_raw_ext("jpg"));
    }
}
