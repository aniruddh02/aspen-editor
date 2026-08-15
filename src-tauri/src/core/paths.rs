use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LIGHTROOM_MCP_VERSION: &str = "v0.9.0";
const NODE_INSTALL_URL: &str = "https://nodejs.org/en/download";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDepsStatus {
    pub lightroom_mcp_ready: bool,
    pub lightroom_mcp_path: Option<String>,
    pub lightroom_mcp_source: String,
    pub node_available: bool,
    pub npx_path: Option<String>,
    pub node_install_url: String,
    pub message: String,
}

pub fn runtime_deps_status() -> RuntimeDepsStatus {
    let mcp = resolve_lightroom_mcp();
    let npx = resolve_npx().ok();
    let node_available = npx.is_some() || resolve_node().is_ok();
    match mcp {
        Ok((path, source)) => RuntimeDepsStatus {
            lightroom_mcp_ready: true,
            lightroom_mcp_path: Some(path.display().to_string()),
            lightroom_mcp_source: source,
            node_available,
            npx_path: npx.map(|p| p.display().to_string()),
            node_install_url: NODE_INSTALL_URL.into(),
            message: "Lightroom MCP helper is ready (Node.js not required).".into(),
        },
        Err(error) => RuntimeDepsStatus {
            lightroom_mcp_ready: false,
            lightroom_mcp_path: None,
            lightroom_mcp_source: "missing".into(),
            node_available,
            npx_path: npx.map(|p| p.display().to_string()),
            node_install_url: NODE_INSTALL_URL.into(),
            message: format!("{error:#}"),
        },
    }
}

/// Prefer bundled/cached standalone MCP binary; fall back to npx only if present.
/// The app bundle on macOS is read-only, so we copy the bundled binary to a
/// writable cache directory before execution.
pub fn resolve_lightroom_mcp() -> anyhow::Result<(PathBuf, String)> {
    if let Some(bundled) = bundled_lightroom_mcp() {
        match copy_to_writable_cache(&bundled) {
            Ok(cached) => return Ok((cached, "bundled".into())),
            Err(e) => {
                tracing::warn!("Could not cache bundled helper: {e}; trying in-place");
                if ensure_executable(&bundled).is_ok() {
                    return Ok((bundled, "bundled".into()));
                }
            }
        }
    }

    if let Ok(path) = ensure_cached_lightroom_mcp() {
        return Ok((path, "cached".into()));
    }

    if let Ok(npx) = resolve_npx() {
        return Ok((npx, "npx".into()));
    }

    anyhow::bail!(
        "ASPEN-LRC-CONNECT-SPAWN: Lightroom helper is unavailable. \
Aspen normally bundles it; if this build is missing resources, install Node.js LTS from \
{NODE_INSTALL_URL} (no Homebrew required), then restart Aspen."
    )
}

/// Copy a bundled binary from the (read-only) app bundle into a writable cache dir.
fn copy_to_writable_cache(source: &Path) -> anyhow::Result<PathBuf> {
    let cache_dir = directories::ProjectDirs::from("com", "aniruddh02", "Aspen")
        .map(|d| d.data_local_dir().join("bin"))
        .ok_or_else(|| anyhow::anyhow!("could not resolve Aspen data directory"))?;
    fs::create_dir_all(&cache_dir)?;
    let dest = cache_dir.join("lightroom-mcp");

    let source_size = fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    let dest_size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);

    if dest.is_file() && dest_size == source_size && dest_size > 0 {
        ensure_executable(&dest)?;
        return Ok(dest);
    }

    fs::copy(source, &dest)?;
    ensure_executable(&dest)?;
    let _ = Command::new("xattr").args(["-cr"]).arg(&dest).status();
    Ok(dest)
}

pub fn lightroom_mcp_args(source: &str) -> Vec<String> {
    if source == "npx" {
        vec!["-y".into(), "@mskalski/lightroom-mcp".into()]
    } else {
        Vec::new()
    }
}

fn bundled_lightroom_mcp() -> Option<PathBuf> {
    // Dev / nearby checkout
    let near_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/lightroom-mcp");
    if near_manifest.is_file() {
        return Some(near_manifest);
    }

    // Packaged app: Aspen.app/Contents/MacOS/aspen → ../Resources/lightroom-mcp
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            let candidate = macos_dir
                .join("../Resources/lightroom-mcp")
                .canonicalize()
                .unwrap_or_else(|_| macos_dir.join("../Resources/lightroom-mcp"));
            if candidate.is_file() {
                return Some(candidate);
            }
            let nested = macos_dir.join("../Resources/resources/lightroom-mcp");
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

fn ensure_cached_lightroom_mcp() -> anyhow::Result<PathBuf> {
    let cache_dir = directories::ProjectDirs::from("com", "aniruddh02", "Aspen")
        .map(|d| d.data_local_dir().join("bin"))
        .ok_or_else(|| anyhow::anyhow!("could not resolve Aspen data directory"))?;
    fs::create_dir_all(&cache_dir)?;
    let dest = cache_dir.join("lightroom-mcp");
    if dest.is_file() {
        ensure_executable(&dest)?;
        return Ok(dest);
    }

    let arch = std::env::consts::ARCH;
    let asset = match arch {
        "aarch64" => "lightroom-mcp-darwin-arm64",
        "x86_64" => "lightroom-mcp-darwin-x64",
        other => anyhow::bail!("unsupported architecture for Lightroom MCP: {other}"),
    };
    let url = format!(
        "https://github.com/Automaat/lightroom-mcp/releases/download/{LIGHTROOM_MCP_VERSION}/{asset}"
    );
    let tmp = dest.with_extension("download");
    let bytes = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?
        .get(&url)
        .send()?
        .error_for_status()?
        .bytes()?;
    fs::write(&tmp, &bytes)?;
    ensure_executable(&tmp)?;
    let _ = Command::new("xattr").args(["-cr"]).arg(&tmp).status();
    fs::rename(&tmp, &dest)?;
    Ok(dest)
}

fn ensure_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        fs::set_permissions(path, perms)?;
    }
    let _ = Command::new("xattr").args(["-cr"]).arg(path).status();
    Ok(())
}

/// Resolve `npx` for GUI apps whose PATH often omits Homebrew / Node installs.
pub fn resolve_npx() -> anyhow::Result<PathBuf> {
    let candidates = [
        "/opt/homebrew/bin/npx",
        "/usr/local/bin/npx",
        "/opt/local/bin/npx",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Ok(path) = which_via_login_shell("npx") {
        if path.is_file() {
            return Ok(path);
        }
    }

    anyhow::bail!("npx not found")
}

pub fn resolve_node() -> anyhow::Result<PathBuf> {
    for candidate in [
        "/opt/homebrew/bin/node",
        "/usr/local/bin/node",
        "/opt/local/bin/node",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    which_via_login_shell("node")
}

fn which_via_login_shell(bin: &str) -> anyhow::Result<PathBuf> {
    let output = Command::new("/bin/zsh")
        .args(["-lic", &format!("command -v {bin}")])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("shell lookup failed for {bin}");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = text.lines().next().unwrap_or("").trim();
    if path.is_empty() {
        anyhow::bail!("{bin} not found in login shell PATH");
    }
    Ok(PathBuf::from(path))
}

/// Open a folder/file in Finder (more reliable than frontend opener for absolute paths).
pub fn open_path(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("ASPEN-FS-OPEN: path does not exist: {}", path.display());
    }
    let status = Command::new("open").arg(path).status()?;
    if !status.success() {
        anyhow::bail!("ASPEN-FS-OPEN: open exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_status_reports_install_url() {
        let status = runtime_deps_status();
        assert!(status.node_install_url.contains("nodejs.org"));
    }

    #[test]
    fn lightroom_args_for_npx_include_package() {
        let args = lightroom_mcp_args("npx");
        assert_eq!(args, vec!["-y", "@mskalski/lightroom-mcp"]);
        assert!(lightroom_mcp_args("bundled").is_empty());
    }
}
