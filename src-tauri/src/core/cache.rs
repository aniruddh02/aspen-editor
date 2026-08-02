use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::core::settings::cache_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub size: u64,
    pub mtime_secs: u64,
    pub blake3: String,
    pub phash: Option<u64>,
    pub preview_w: Option<u32>,
    pub preview_h: Option<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HashCache {
    entries: HashMap<String, CacheEntry>,
}

impl HashCache {
    pub fn load() -> Self {
        let Some(path) = cache_path() else {
            return Self::default();
        };
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = cache_path() else {
            anyhow::bail!("could not resolve cache path");
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn clear() -> anyhow::Result<()> {
        if let Some(path) = cache_path() {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn get_valid(&self, path: &Path, size: u64, mtime: SystemTime) -> Option<&CacheEntry> {
        let key = path.to_string_lossy().to_string();
        let mtime_secs = mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs();
        let entry = self.entries.get(&key)?;
        if entry.size == size && entry.mtime_secs == mtime_secs {
            Some(entry)
        } else {
            None
        }
    }

    pub fn insert(&mut self, path: &Path, entry: CacheEntry) {
        self.entries
            .insert(path.to_string_lossy().to_string(), entry);
    }
}

pub fn file_meta(path: &Path) -> anyhow::Result<(u64, SystemTime)> {
    let meta = fs::metadata(path)?;
    Ok((meta.len(), meta.modified()?))
}

pub fn blake3_file(path: &Path) -> anyhow::Result<String> {
    let bytes = fs::read(path)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

pub fn resolve_blake3(cache: &mut HashCache, path: &Path) -> anyhow::Result<String> {
    let (size, mtime) = file_meta(path)?;
    if let Some(hit) = cache.get_valid(path, size, mtime) {
        return Ok(hit.blake3.clone());
    }
    let hash = blake3_file(path)?;
    let mtime_secs = mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    cache.insert(
        path,
        CacheEntry {
            size,
            mtime_secs,
            blake3: hash.clone(),
            phash: None,
            preview_w: None,
            preview_h: None,
        },
    );
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn blake3_stable() {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), b"hello aspen").unwrap();
        let a = blake3_file(f.path()).unwrap();
        let b = blake3_file(f.path()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn cache_hit_and_miss_on_change() {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), b"v1").unwrap();
        let mut cache = HashCache::default();
        let h1 = resolve_blake3(&mut cache, f.path()).unwrap();
        let h2 = resolve_blake3(&mut cache, f.path()).unwrap();
        assert_eq!(h1, h2);
        // Different size forces cache miss even if mtime resolution is coarse
        fs::write(f.path(), b"v2-longer-content").unwrap();
        let h3 = resolve_blake3(&mut cache, f.path()).unwrap();
        assert_ne!(h1, h3);
    }
}
