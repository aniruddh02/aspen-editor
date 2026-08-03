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
        // Finder tags are best-effort; failures do not invalidate the file operation.
        let result = Command::new("xattr")
            .args([
                "-w",
                "com.apple.metadata:_kMDItemUserTags",
                &format!("(\"{tag}\")"),
                path.to_str().unwrap_or(""),
            ])
            .status();
        match result {
            Ok(status) if status.success() => {}
            Ok(status) => crate::core::logging::record(
                crate::core::logging::LogEvent::new(
                    crate::core::logging::LogLevel::Warn,
                    "deduplicate",
                    "finder_tag",
                    "file-action",
                    "tag.apply",
                    "Finder color tag command failed",
                )
                .with_error("ASPEN-FS-TAG", format!("xattr exited with {status}")),
            ),
            Err(error) => crate::core::logging::record(
                crate::core::logging::LogEvent::new(
                    crate::core::logging::LogLevel::Warn,
                    "deduplicate",
                    "finder_tag",
                    "file-action",
                    "tag.apply",
                    "Finder color tag could not be applied",
                )
                .with_error("ASPEN-FS-TAG", error.to_string()),
            ),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, color);
    }
}
