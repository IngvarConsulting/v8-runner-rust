//! A source set's tree as git sees it (#575).
//!
//! The answer of git — the index and what `git status` reports — comes in already parsed
//! ([`GitWorktree`]); this module spawns nothing. A file git reports clean has the blob id of
//! its index entry; a file git reports changed or untracked, an `assume-unchanged` or
//! `skip-worktree` file and a file in an unfinished merge are hashed by the caller's
//! [`BlobHasher`] — `git hash-object`, so the id is the one `git add` would store. The files are
//! selected by the predicates of the push walk ([`scanner::is_push_source_rel_path`]), so the
//! listing equals the walk's (`git_and_scan_snapshots_agree`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::change_detection::file_state::mtime_nanos;
use crate::change_detection::scanner;
use crate::domain::git_worktree::{GitIndexEntry, GitWorktree, ScanReason};

/// Why the caller's blob hasher gave no ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobHashError {
    /// The command was cancelled; the hashing process is gone.
    Interrupted,
    /// Hashing failed; the reason is for the log.
    Failed(String),
}

/// Hashes files named from the root of the working tree as `git add` would store them, the ids
/// in the order of the paths. It asks the interruption predicate it is given.
pub type BlobHasher<'h> =
    dyn FnMut(&[String], &mut dyn FnMut() -> bool) -> Result<Vec<String>, BlobHashError> + 'h;

/// Where a source set lies in a working tree, and the tree's parsed answer.
pub struct GitSource<'v, 'h> {
    pub view: &'v GitWorktree,
    /// The set root from the root of the working tree, `/`-separated; empty — the root itself.
    pub root: String,
    /// `workPath` below the set root, `/`-separated, when it lies there.
    pub work_path: Option<String>,
    pub hash_blobs: &'h mut BlobHasher<'h>,
}

/// One file of the set as git sees it.
#[derive(Debug, Clone)]
pub(super) struct ObservedFile {
    /// From the set root, `/`-separated.
    pub(super) rel_path: String,
    pub(super) path: PathBuf,
    /// The blob id; `None` when the file changed while it was hashed: it is a change, and its
    /// id is not remembered.
    pub(super) hash: Option<String>,
}

/// Why git's answer cannot describe the set.
#[derive(Debug, Clone)]
pub(super) enum ObserveError {
    Interrupted,
    Unusable { reason: ScanReason, detail: String },
}

impl<'v, 'h> GitSource<'v, 'h> {
    /// The path below the set root of a path from the root of the working tree, if it lies
    /// there and is not in `workPath`.
    pub(super) fn below_root<'p>(&self, path: &'p str) -> Option<&'p str> {
        let below = if self.root.is_empty() {
            path
        } else {
            let head = path.get(..self.root.len())?;
            let same = if self.view.ignore_case {
                head.eq_ignore_ascii_case(&self.root)
            } else {
                head == self.root
            };
            if !same || path.as_bytes().get(self.root.len()) != Some(&b'/') {
                return None;
            }
            &path[self.root.len() + 1..]
        };
        let in_work_path = self.work_path.as_deref().is_some_and(|work| {
            below.len() > work.len()
                && below.as_bytes()[work.len()] == b'/'
                && if self.view.ignore_case {
                    below[..work.len()].eq_ignore_ascii_case(work)
                } else {
                    &below[..work.len()] == work
                }
        });
        (!in_work_path).then_some(below)
    }

    fn in_toplevel(&self, below: &str) -> String {
        if self.root.is_empty() {
            below.to_owned()
        } else {
            format!("{}/{below}", self.root)
        }
    }

    /// Every source file of the set with its blob id, in path order.
    pub(super) fn observe(
        &mut self,
        set_root: &Path,
        interrupted: &mut dyn FnMut() -> bool,
    ) -> Result<Vec<ObservedFile>, ObserveError> {
        let unusable =
            |reason: ScanReason, detail: String| ObserveError::Unusable { reason, detail };
        // Stages of one path together: an unfinished merge has several.
        let mut entries: BTreeMap<String, Vec<&GitIndexEntry>> = BTreeMap::new();
        for entry in &self.view.index {
            let Some(below) = self.below_root(&entry.path) else {
                continue;
            };
            if entry.mode == GitIndexEntry::GITLINK_MODE {
                if scanner::is_push_source_rel_path(below) {
                    return Err(unusable(
                        ScanReason::NestedRepository,
                        format!("'{}' is a submodule inside the set", entry.path),
                    ));
                }
                continue;
            }
            if scanner::is_push_source_rel_path(below) {
                entries.entry(below.to_owned()).or_default().push(entry);
            }
        }

        let mut files: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut to_hash: Vec<String> = Vec::new();
        for (below, stages) in &entries {
            let full = self.in_toplevel(below);
            let clean = stages.len() == 1
                && stages[0].stage == 0
                && !stages[0].flagged
                && !self.view.worktree_changed.contains(&full);
            if clean {
                // A link is no source for the walk, and git says the tree still holds it.
                if stages[0].mode != GitIndexEntry::SYMLINK_MODE {
                    files.insert(below.clone(), Some(stages[0].oid.clone()));
                }
            } else {
                to_hash.push(below.clone());
            }
        }
        for path in &self.view.untracked {
            let Some(below) = self.below_root(path.trim_end_matches('/')) else {
                continue;
            };
            if path.ends_with('/') {
                // With `--untracked-files=all` git names a directory only for a nested
                // repository, whose files it does not list.
                if scanner::is_push_source_rel_path(&format!("{below}/x")) {
                    return Err(unusable(
                        ScanReason::NestedRepository,
                        format!("'{path}' is a repository inside the set"),
                    ));
                }
                continue;
            }
            if scanner::is_push_source_rel_path(below) {
                to_hash.push(below.to_owned());
            }
        }

        // Read directly: what is a regular file now, and its time before the hash.
        let mut listed = Vec::new();
        for below in to_hash {
            if interrupted() {
                return Err(ObserveError::Interrupted);
            }
            let path = set_root.join(&below);
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_file() => {
                    listed.push((below, modified_ns(&path, &meta)));
                }
                // A link or a directory is no source; the walk skips it as well.
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(unusable(
                        ScanReason::GitFailed,
                        format!("'{}' cannot be read: {error}", path.display()),
                    ))
                }
            }
        }
        let names: Vec<String> = listed
            .iter()
            .map(|(below, _)| self.in_toplevel(below))
            .collect();
        let ids = (self.hash_blobs)(&names, interrupted).map_err(|error| match error {
            BlobHashError::Interrupted => ObserveError::Interrupted,
            BlobHashError::Failed(detail) => unusable(ScanReason::GitFailed, detail),
        })?;
        for ((below, before), id) in listed.into_iter().zip(ids) {
            let after = std::fs::symlink_metadata(set_root.join(&below))
                .ok()
                .and_then(|meta| modified_ns(&set_root.join(&below), &meta));
            // A file whose time moved while it was hashed may hold another content than the id.
            let unmoved = before.is_some() && before == after;
            files.insert(below, unmoved.then_some(id));
        }
        if interrupted() {
            return Err(ObserveError::Interrupted);
        }
        Ok(files
            .into_iter()
            .map(|(rel_path, hash)| ObservedFile {
                path: set_root.join(&rel_path),
                rel_path,
                hash,
            })
            .collect())
    }
}

fn modified_ns(path: &Path, meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|time| mtime_nanos(time, path).ok())
}

/// Memory keys as git spells them: with `core.ignorecase` a path git names in other letter
/// case is the remembered file (#575).
pub(super) struct Spelling<'m> {
    remembered: &'m HashMap<String, crate::change_detection::hash_storage::StoredFileState>,
    folded: HashMap<String, &'m str>,
}

impl<'m> Spelling<'m> {
    pub(super) fn new(
        remembered: &'m HashMap<String, crate::change_detection::hash_storage::StoredFileState>,
        ignore_case: bool,
    ) -> Self {
        let folded = if ignore_case {
            remembered
                .keys()
                .map(|key| (key.to_lowercase(), key.as_str()))
                .collect()
        } else {
            HashMap::new()
        };
        Self { remembered, folded }
    }

    /// The remembered key of `path`, if the memory holds it.
    pub(super) fn of(&self, path: &str) -> Option<&'m str> {
        self.remembered
            .get_key_value(path)
            .map(|(key, _)| key.as_str())
            .or_else(|| self.folded.get(&path.to_lowercase()).copied())
    }
}
