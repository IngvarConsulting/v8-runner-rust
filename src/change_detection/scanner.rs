use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

use crate::change_detection::file_state::{mtime_nanos, MtimeError};

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("source scan interrupted")]
    Interrupted,

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

/// An ignored file, or the temporary file of its atomic replacement left by a killed process.
fn is_ignored_file(name: &str) -> bool {
    IGNORED_FILES.iter().any(|ignored| {
        name.strip_prefix(ignored).is_some_and(|rest| {
            rest.is_empty() || rest.starts_with(crate::support::fs::ATOMIC_WRITE_CANDIDATE_SUFFIX)
        })
    })
}

/// One fully read source file. Metadata never substitutes for its content hash.
#[derive(Debug, Clone)]
pub struct HashedFile {
    pub path: PathBuf,
    pub rel_path: String,
    pub mtime_ns: u64,
    pub hash: String,
}

/// Full scanner output for one source-set root.
#[derive(Debug, Clone)]
pub struct ScanSnapshot {
    pub scan_started_at: u64,
    pub files: Vec<HashedFile>,
}

/// Hash every selected regular file, including equal-size edits with exactly the old
/// remembered mtime (#447). The predicate is the caller's existing interruption policy;
/// successful post-publication bookkeeping supplies an always-false predicate.
pub fn scan(root: &Path, interrupted: &mut dyn FnMut() -> bool) -> Result<ScanSnapshot, ScanError> {
    check_interruption(interrupted)?;
    let scan_started_at =
        mtime_nanos(std::time::SystemTime::now(), root).map_err(|source| ScanError::Mtime {
            path: root.to_path_buf(),
            source,
        })?;
    let mut files = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_ignored_dir(e))
    {
        check_interruption(interrupted)?;
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
            if is_ignored_file(name) {
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
        let hash = hash_file(path, interrupted)?;
        files.push(HashedFile {
            path: path.to_path_buf(),
            rel_path,
            mtime_ns,
            hash,
        });
    }

    check_interruption(interrupted)?;
    Ok(ScanSnapshot {
        scan_started_at,
        files,
    })
}

/// Compute SHA-256 hex digest of a file's contents.
pub fn hash_file(path: &Path, interrupted: &mut dyn FnMut() -> bool) -> Result<String, ScanError> {
    check_interruption(interrupted)?;
    let file = std::fs::File::open(path).map_err(|e| ScanError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    hash_reader(file, path, interrupted)
}

fn hash_reader(
    mut reader: impl Read,
    path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<String, ScanError> {
    // Working storage, not a maximum file size. Files of any size are streamed.
    let mut buffer = [0u8; 64 * 1024];
    let mut digest = Sha256::new();
    loop {
        check_interruption(interrupted)?;
        let length = match reader.read(&mut buffer) {
            Ok(length) => length,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(ScanError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        check_interruption(interrupted)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn check_interruption(interrupted: &mut dyn FnMut() -> bool) -> Result<(), ScanError> {
    if interrupted() {
        Err(ScanError::Interrupted)
    } else {
        Ok(())
    }
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
    use super::{scan, ScanSnapshot};
    use crate::change_detection::file_state::mtime_nanos;
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
            .files
            .iter()
            .map(|candidate| candidate.rel_path.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// Every selected file is read, including old files and an unchanged mtime.
    #[test]
    fn every_selected_file_is_hashed_regardless_of_mtime() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let old = SystemTime::now() - Duration::from_secs(600);
        write_touched(&root.join("Old.bsl"), "old", old);
        write_touched(&root.join("Near.bsl"), "near", SystemTime::now());
        write_touched(&root.join("New.bsl"), "new", old);
        let snapshot = scan(root, &mut || false).expect("scan");
        assert_eq!(candidates(&snapshot), ["Near.bsl", "New.bsl", "Old.bsl"]);
        for file in &snapshot.files {
            assert_eq!(
                file.hash,
                super::hash_file(&file.path, &mut || false).expect("hash")
            );
        }
    }

    /// A restored copy whose mtime moved backwards remains in the full content scan.
    #[test]
    fn a_restored_copy_with_an_old_mtime_is_hashed() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        write_touched(
            &root.join("Module.bsl"),
            "restored bytes",
            SystemTime::now() - Duration::from_secs(600),
        );
        let snapshot = scan(root, &mut || false).expect("scan");
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
    }

    /// Служебные и порождённые каталоги и файл состояния выгрузки в обход не входят.
    #[test]
    fn service_and_generated_paths_are_never_scanned() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("Module.bsl"), "module").expect("module");
        fs::write(root.join("ConfigDumpInfo.xml"), "<info/>").expect("dump info");
        fs::write(root.join("ConfigDumpInfo.xml.candidate-x1"), "<info/>").expect("left write");
        for ignored in [
            ".git", ".gradle", "build", "target", "temp", "tmp", ".yaxunit",
        ] {
            let nested = root.join(ignored).join("nested");
            fs::create_dir_all(&nested).expect("ignored dir");
            fs::write(nested.join("File.bsl"), "generated").expect("ignored file");
        }

        let snapshot = scan(root, &mut || false).expect("scan");

        let seen: Vec<&str> = snapshot
            .files
            .iter()
            .map(|file| file.rel_path.as_str())
            .collect();
        assert_eq!(seen, ["Module.bsl"]);
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
    }

    /// Корень набора сканируется всегда, даже названный как служебный каталог; служебные
    /// каталоги внутри него пропускаются.
    #[test]
    fn selected_roots_are_scanned_while_ignored_descendants_stay_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        for root_name in super::IGNORED_DIRS {
            let root = dir.path().join(root_name);
            std::fs::create_dir(&root).expect("root");
            std::fs::write(root.join("Module.bsl"), "source").expect("source");
            for child_name in super::IGNORED_DIRS {
                let child = root.join(child_name);
                std::fs::create_dir(&child).expect("ignored child");
                std::fs::write(child.join("Module.bsl"), "generated").expect("child source");
            }
            let scanned = scan(&root, &mut || false).expect("scan");
            assert_eq!(scanned.files.len(), 1, "root {root_name}");
            assert_eq!(scanned.files[0].rel_path, "Module.bsl");
        }
    }
    /// Equal file size and the exact remembered mtime do not hide different bytes.
    #[test]
    fn changed_bytes_with_the_exact_remembered_old_mtime_are_hashed() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let module = root.join("Module.bsl");
        let watermark = SystemTime::now();
        write_touched(
            &module,
            "before bytes",
            watermark - Duration::from_secs(600),
        );
        let before = fs::metadata(&module).expect("before metadata");
        let remembered = before.modified().expect("before mtime");
        let remembered_ns = mtime_nanos(remembered, &module).expect("mtime nanos");
        let old_hash = super::hash_file(&module, &mut || false).expect("old hash");

        write_touched(&module, "edited bytes", remembered);
        let after = fs::metadata(&module).expect("after metadata");
        assert_eq!(before.len(), after.len(), "size is unchanged");
        assert_eq!(remembered, after.modified().expect("after mtime"));
        assert!(remembered_ns < mtime_nanos(watermark, root).expect("watermark") - 2_000_000_000);

        let snapshot = scan(root, &mut || false).expect("scan");
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
        assert_eq!(snapshot.files[0].mtime_ns, remembered_ns);
        assert_ne!(snapshot.files[0].hash, old_hash, "changed bytes are read");
        assert_eq!(
            snapshot.files[0].hash,
            super::hash_file(&module, &mut || false).expect("edited hash")
        );
    }
    /// Buffer capacity bounds memory, never accepted file size.
    #[test]
    fn hashing_streams_files_larger_than_its_working_buffer() {
        use std::io::{Read, Result};
        struct LargeReader {
            left: usize,
            largest: usize,
        }
        impl Read for LargeReader {
            fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
                self.largest = self.largest.max(buffer.len());
                let count = self.left.min(buffer.len());
                buffer[..count].fill(17);
                self.left -= count;
                Ok(count)
            }
        }
        let length = 3 * 64 * 1024 + 7;
        let mut reader = LargeReader {
            left: length,
            largest: 0,
        };
        let hash =
            super::hash_reader(&mut reader, Path::new("large"), &mut || false).expect("hash");
        use sha2::{Digest, Sha256};
        assert_eq!(hash, format!("{:x}", Sha256::digest(vec![17; length])));
        assert_eq!(reader.left, 0);
        assert!(reader.largest <= 64 * 1024);
    }

    /// A partially read file never yields a digest on IO failure or cancellation.
    #[test]
    fn partial_reads_fail_without_a_hash_and_cancel_before_the_next_read() {
        use std::{
            cell::Cell,
            io::{self, Read},
            rc::Rc,
        };
        struct PartialReader {
            reads: Rc<Cell<usize>>,
            cancelled: Rc<Cell<bool>>,
            cancel: bool,
        }
        impl Read for PartialReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.reads.get();
                self.reads.set(count + 1);
                if count > 0 {
                    return Err(io::Error::other("read failed after a prefix"));
                }
                buffer[..3].copy_from_slice(b"abc");
                self.cancelled.set(self.cancel);
                Ok(3)
            }
        }
        for cancel in [false, true] {
            let reads = Rc::new(Cell::new(0));
            let cancelled = Rc::new(Cell::new(false));
            let reader = PartialReader {
                reads: reads.clone(),
                cancelled: cancelled.clone(),
                cancel,
            };
            let error = super::hash_reader(reader, Path::new("partial"), &mut || cancelled.get())
                .expect_err("no partial hash");
            if cancel {
                assert!(matches!(error, super::ScanError::Interrupted));
                assert_eq!(reads.get(), 1, "no read after cancellation");
            } else {
                assert!(
                    matches!(error, super::ScanError::Read { path, .. } if path == Path::new("partial"))
                );
                assert_eq!(reads.get(), 2);
            }
        }
    }

    #[test]
    fn interrupted_io_is_retried_and_empty_files_have_the_complete_digest() {
        use std::io::{self, Read};
        struct OnceInterrupted(bool);
        impl Read for OnceInterrupted {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    self.0 = false;
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Ok(0)
                }
            }
        }
        let digest = super::hash_reader(OnceInterrupted(true), Path::new("empty"), &mut || false)
            .expect("retry then EOF");
        assert_eq!(
            digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn an_interrupted_scan_returns_no_partial_snapshot() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("A.bsl"), "first").expect("first");
        fs::write(dir.path().join("B.bsl"), "second").expect("second");
        // Root, first file and its read finish before the second directory entry.
        let mut checkpoints = 0;
        let error = scan(dir.path(), &mut || {
            checkpoints += 1;
            checkpoints >= 9
        })
        .expect_err("cancelled");
        assert!(matches!(error, super::ScanError::Interrupted));
        assert_eq!(checkpoints, 9);
    }
}
