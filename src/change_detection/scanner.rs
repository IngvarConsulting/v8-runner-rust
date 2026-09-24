use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

use crate::change_detection::file_state::{mtime_nanos, MtimeError};

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("failed to walk directory '{path}': {source}")]
    Walk {
        path: PathBuf,
        source: walkdir::Error,
    },

    #[error("failed to read file '{path}': {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read metadata for '{path}': {source}")]
    Meta {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to convert mtime for '{path}': {source}")]
    Mtime { path: PathBuf, source: MtimeError },

    #[error("failed to build path relative to scan root '{root}' for '{path}'")]
    RelativePath { root: PathBuf, path: PathBuf },
}

/// Directory/file names that are always excluded from scanning.
const IGNORED_DIRS: &[&str] = &[
    ".git", ".gradle", "build", "target", "temp", "tmp", ".yaxunit",
];
const IGNORED_FILES: &[&str] = &["ConfigDumpInfo.xml"];

/// Coarse filesystem mtime guard (2 seconds).
pub const COARSE_MARGIN_NS: u64 = 2_000_000_000;

/// One discovered source file (metadata only, no hash).
#[derive(Debug, Clone)]
pub struct SeenFile {
    pub rel_path: String,
    pub mtime_ns: u64,
}

/// One hashed candidate file.
#[derive(Debug, Clone)]
pub struct CandidateFile {
    pub path: PathBuf,
    pub rel_path: String,
    pub mtime_ns: u64,
    pub hash: String,
}

/// Full scanner output for one source-set root.
#[derive(Debug, Clone)]
pub struct ScanSnapshot {
    pub scan_started_at: u64,
    pub seen_files: Vec<SeenFile>,
    pub candidates: Vec<CandidateFile>,
}

/// Recursively scan `root` and return:
/// - all seen files with metadata
/// - only candidate files hashed by mtime/watermark rules
pub fn scan(
    root: &Path,
    watermark: Option<u64>,
    stored_keys: &HashSet<String>,
) -> Result<ScanSnapshot, ScanError> {
    let scan_started_at =
        mtime_nanos(std::time::SystemTime::now(), root).map_err(|source| ScanError::Mtime {
            path: root.to_path_buf(),
            source,
        })?;
    let mut seen_files = Vec::new();
    let mut candidates = Vec::new();

    let cutoff = watermark.map(|w| w.saturating_sub(COARSE_MARGIN_NS));
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_ignored_dir(e))
    {
        let entry = entry.map_err(|e| ScanError::Walk {
            path: root.to_path_buf(),
            source: e,
        })?;

        let path = entry.path();

        if entry.file_type().is_dir() {
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        // Skip ignored file names.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if IGNORED_FILES.contains(&name) {
                continue;
            }
        }

        let meta = std::fs::metadata(path).map_err(|e| ScanError::Meta {
            path: path.to_path_buf(),
            source: e,
        })?;

        let mtime_ns = mtime_nanos(
            meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            path,
        )
        .map_err(|source| ScanError::Mtime {
            path: path.to_path_buf(),
            source,
        })?;
        let rel_path = rel_path(root, path)?;
        let seen = SeenFile {
            rel_path: rel_path.clone(),
            mtime_ns,
        };
        let is_new = !stored_keys.contains(&rel_path);
        let is_candidate = match cutoff {
            None => true,
            Some(cutoff) => is_new || mtime_ns >= cutoff,
        };
        if is_candidate {
            let hash = hash_file(path)?;
            candidates.push(CandidateFile {
                path: path.to_path_buf(),
                rel_path,
                mtime_ns,
                hash,
            });
        }
        seen_files.push(seen);
    }

    Ok(ScanSnapshot {
        scan_started_at,
        seen_files,
        candidates,
    })
}

/// Compute SHA-256 hex digest of a file's contents.
pub fn hash_file(path: &Path) -> Result<String, ScanError> {
    let data = std::fs::read(path).map_err(|e| ScanError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    let digest = Sha256::digest(&data);
    Ok(format!("{:x}", digest))
}

fn rel_path(root: &Path, path: &Path) -> Result<String, ScanError> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| ScanError::RelativePath {
            root: root.to_path_buf(),
            path: path.to_path_buf(),
        })?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn is_ignored_dir(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let Some(name) = entry.file_name().to_str() else {
        return false;
    };
    IGNORED_DIRS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::{scan, ScanSnapshot, COARSE_MARGIN_NS};
    use crate::change_detection::file_state::mtime_nanos;
    use std::collections::HashSet;
    use std::fs::{self, File};
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;

    fn write_touched(path: &Path, contents: &str, modified: SystemTime) {
        fs::write(path, contents).expect("write");
        File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_modified(modified)
            .expect("set mtime");
    }

    fn candidates(snapshot: &ScanSnapshot) -> Vec<&str> {
        let mut names: Vec<&str> = snapshot
            .candidates
            .iter()
            .map(|candidate| candidate.rel_path.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// Известный прошлому снимку файл хешируется, только если тронут не раньше водяного
    /// знака за вычетом запаса: грубые часы файловой системы правку не прячут, а нетронутое
    /// не читается. Новый файл хешируется всегда.
    #[test]
    fn a_known_file_is_hashed_only_when_touched_within_the_margin() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let margin = Duration::from_nanos(COARSE_MARGIN_NS);
        let watermark = SystemTime::now() - 10 * margin;
        write_touched(&root.join("Old.bsl"), "old", watermark - 2 * margin);
        write_touched(&root.join("Near.bsl"), "near", watermark - margin / 2);
        write_touched(&root.join("New.bsl"), "new", watermark - 2 * margin);
        let known: HashSet<String> = ["Old.bsl", "Near.bsl"].map(str::to_owned).into();
        let watermark_ns = mtime_nanos(watermark, root).expect("watermark");

        let snapshot = scan(root, Some(watermark_ns), &known).expect("scan");

        assert_eq!(snapshot.seen_files.len(), 3);
        assert_eq!(candidates(&snapshot), ["Near.bsl", "New.bsl"]);
    }

    /// Служебные и порождённые каталоги и файл состояния выгрузки в обход не входят.
    #[test]
    fn service_and_generated_paths_are_never_scanned() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("Module.bsl"), "module").expect("module");
        fs::write(root.join("ConfigDumpInfo.xml"), "<info/>").expect("dump info");
        for ignored in [
            ".git", ".gradle", "build", "target", "temp", "tmp", ".yaxunit",
        ] {
            let nested = root.join(ignored).join("nested");
            fs::create_dir_all(&nested).expect("ignored dir");
            fs::write(nested.join("File.bsl"), "generated").expect("ignored file");
        }

        let snapshot = scan(root, None, &HashSet::new()).expect("scan");

        let seen: Vec<&str> = snapshot
            .seen_files
            .iter()
            .map(|file| file.rel_path.as_str())
            .collect();
        assert_eq!(seen, ["Module.bsl"]);
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
    }
}
