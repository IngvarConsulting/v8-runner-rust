//! The registry of live Designer agents of this OS account: its directory and its lock under
//! the account's state directory (`support::machine::account`). What the account is, where its
//! state lies and how a record is keyed stays in `support`; what the agents' records are and
//! how they are locked is decided here.

use std::path::{Path, PathBuf};

use crate::support::fs::{acquire_advisory_lock, AdvisoryLockGuard};
use crate::support::machine::account::ensure_private_dir;

/// The directory of live Designer agents under the account's state directory `state`,
/// created owner-only when missing.
pub fn agents_dir(state: &Path) -> std::io::Result<PathBuf> {
    let dir = agents_path(state);
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// The account's live-agent directory, `<state>/agents`, without creating it: for readers
/// that must not leave a state behind.
pub fn agents_path(state: &Path) -> PathBuf {
    state.join("agents")
}

/// Takes the account's agent registry lock, `<state>/agents/registry.lock`, waiting for
/// another holder. The lock is not reentrant: callers hold it only to read and write records,
/// never across a start, and call nothing under it that takes it again.
pub fn lock_agent_registry(state: &Path) -> std::io::Result<AdvisoryLockGuard> {
    acquire_advisory_lock(&agents_dir(state)?.join("registry.lock"))
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use crate::support::machine::account::account_key;

    #[test]
    fn the_registry_lock_is_held_by_one_holder_at_a_time() {
        let state = tempfile::tempdir().unwrap();
        let held = lock_agent_registry(state.path()).unwrap();
        let path = state.path().join("agents").join("registry.lock");
        assert_eq!(
            crate::support::fs::try_acquire_advisory_lock(&path)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(held);
        drop(crate::support::fs::try_acquire_advisory_lock(&path).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(state.path().join("agents"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    /// A record is keyed while the registry is held: making the account's salt on first use
    /// must not wait for that same lock.
    #[test]
    fn a_key_is_made_under_the_registry_lock() {
        let state = tempfile::tempdir().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let dir = state.path().to_path_buf();
        std::thread::spawn(move || {
            let _registry = lock_agent_registry(&dir).unwrap();
            let key = account_key(&dir, &["ib", "8.3.27", "user"]);
            let _ = done.send(key.map(|key| key.len()).map_err(|error| error.to_string()));
        });
        let answer = finished
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("making a key under the registry lock never ends");
        assert_eq!(answer, Ok(64));
    }
}
