//! sha256 hashes for caching and staleness (spec §10).

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The hash of a JSON value, as `sha256:<hex>`.
pub fn of_json(v: &serde_json::Value) -> String {
    format!("sha256:{}", sha256(v.to_string().as_bytes()))
}

/// Hashes a file's contents by streaming it.
pub fn file_sha256(path: &Path) -> Result<String, String> {
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("can't read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Hashes every file under a folder (a `.pixellab` document), by relative
/// path and contents, in sorted order.
pub fn dir_sha256(dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for rel in files {
        let bytes = std::fs::read(dir.join(&rel))
            .map_err(|e| format!("can't read {}: {e}", dir.join(&rel).display()))?;
        h.update(rel.as_bytes());
        h.update([0]);
        h.update(sha256(&bytes).as_bytes());
    }
    Ok(hex(&h.finalize()))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("can't read {}: {e}", dir.display()))?;
    for entry in rd {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            collect(root, &p, out)?;
        } else {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// Media hashes cached by path, size and modification time, so large takes
/// aren't re-read on every run.
pub struct HashCache {
    path: PathBuf,
    entries: BTreeMap<String, (u64, u128, String)>,
    dirty: bool,
}

impl HashCache {
    pub fn open(path: PathBuf) -> HashCache {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        HashCache {
            path,
            entries,
            dirty: false,
        }
    }

    pub fn file(&mut self, file: &Path) -> Result<String, String> {
        let meta =
            std::fs::metadata(file).map_err(|e| format!("can't read {}: {e}", file.display()))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let key = file.to_string_lossy().into_owned();
        if let Some((size, m, h)) = self.entries.get(&key) {
            if *size == meta.len() && *m == mtime {
                return Ok(h.clone());
            }
        }
        let h = file_sha256(file)?;
        self.entries.insert(key, (meta.len(), mtime, h.clone()));
        self.dirty = true;
        Ok(h)
    }

    pub fn save(&self) {
        if self.dirty {
            if let Some(d) = self.path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(
                &self.path,
                serde_json::to_string(&self.entries).unwrap_or_default(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digest() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
