use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum FolderColor {
    Green,
    Red,
}

/// Apply Finder color tag via `xattr` user tags (best-effort on macOS).
pub fn apply_folder_tag(path: &Path, color: FolderColor) {
    #[cfg(target_os = "macos")]
    {
        let tag = match color {
            FolderColor::Green => "Green\n2",
            FolderColor::Red => "Red\n6",
        };
        // Write plist-ish tag via xattr; ignore failures (unsigned / sandbox).
        let _ = Command::new("xattr")
            .args([
                "-w",
                "com.apple.metadata:_kMDItemUserTags",
                &format!("(\"{tag}\")"),
                path.to_str().unwrap_or(""),
            ])
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, color);
    }
}
