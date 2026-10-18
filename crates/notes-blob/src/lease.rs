use super::{BlobStoreError, sync_directory};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub(super) struct DeletionGate {
    leases: Mutex<usize>,
}

/// Keeps canonical files available to a backup while it captures and copies
/// them. Dropping a canceled backup's guard releases its lease automatically.
/// Only clones of the acquiring BlobStore share this runtime-local protection.
#[derive(Debug)]
pub struct DeletionLease {
    gate: Arc<DeletionGate>,
}

impl DeletionGate {
    pub(super) fn acquire(self: &Arc<Self>) -> DeletionLease {
        let mut leases = self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *leases = leases
            .checked_add(1)
            .expect("blob deletion lease count overflow");
        DeletionLease { gate: self.clone() }
    }

    pub(super) fn is_leased(&self) -> bool {
        *self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            > 0
    }

    pub(super) fn try_remove(
        &self,
        path: &Path,
        operation: &'static str,
    ) -> Result<bool, BlobStoreError> {
        // Acquisition and actual unlink share this short gate. Verification
        // happens before it; a lease admitted during verification still wins.
        // Never wait for a backup lease while a caller owns the SQLite writer.
        let leases = self
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *leases > 0 {
            return Ok(false);
        }
        let removed = fs::remove_file(path);
        // Unlink is the ordering point. Flushing its directory must not keep a
        // backup's lease acquisition waiting behind filesystem I/O.
        drop(leases);
        match removed {
            Ok(()) => {
                sync_directory(path.parent().expect("canonical blob has a shard"))?;
                Ok(true)
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(BlobStoreError::Io {
                operation,
                path: path.to_owned(),
                source,
            }),
        }
    }
}

impl Drop for DeletionLease {
    fn drop(&mut self) {
        let mut leases = self
            .gate
            .leases
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *leases -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlobHash, BlobStore};
    use std::future::Future;
    use std::sync::{Barrier, mpsc};
    use std::task::{Context, Waker};
    use std::time::Duration;

    fn installed(store: &BlobStore) -> BlobHash {
        let bytes = b"retained during backup";
        let hash = BlobHash::digest(bytes);
        store.install_reader(bytes.as_slice(), hash, 1024).unwrap();
        hash
    }

    #[test]
    fn clones_and_multiple_leases_defer_both_removal_paths_until_the_last_guard_drops() {
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path());
        let clone = store.clone();
        let hash = installed(&store);
        let first = store.lease_deletions();
        let second = clone.lease_deletions();
        assert!(!clone.remove_verified(hash, 1024).unwrap());
        assert!(!clone.discard_unverified(hash).unwrap());
        drop(first);
        assert!(!store.remove_verified(hash, 1024).unwrap());
        drop(second);
        assert!(store.remove_verified(hash, 1024).unwrap());
        assert!(!clone.discard_unverified(hash).unwrap());
    }

    #[test]
    fn a_removal_does_not_wait_for_a_backup_guard_to_drop() {
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path());
        let hash = installed(&store);
        let lease = store.lease_deletions();
        let (done_tx, done_rx) = mpsc::channel();
        let worker =
            std::thread::spawn(move || done_tx.send(store.remove_verified(hash, 1024)).unwrap());
        assert!(
            !done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("removal defers without waiting for lease drop")
                .unwrap()
        );
        drop(lease);
        worker.join().unwrap();
    }

    #[test]
    fn dropping_a_pending_backup_future_releases_its_lease() {
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path());
        let hash = installed(&store);
        let leased = store.clone();
        let mut backup = Box::pin(async move {
            let _lease = leased.lease_deletions();
            std::future::pending::<()>().await;
        });
        assert!(
            backup
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert!(!store.remove_verified(hash, 1024).unwrap());
        drop(backup);
        assert!(store.remove_verified(hash, 1024).unwrap());
    }

    #[test]
    fn a_lease_admitted_after_verification_still_prevents_unlink() {
        let directory = tempfile::tempdir().unwrap();
        let store = BlobStore::new(directory.path());
        let hash = installed(&store);
        let verified = store.open_verified(hash, 1024).unwrap();
        let path = verified.blob.path.clone();
        drop(verified);
        let lease = store.clone().lease_deletions();
        assert!(!store.deletions.try_remove(&path, "test unlink").unwrap());
        assert!(path.exists());
        drop(lease);
        assert!(store.deletions.try_remove(&path, "test unlink").unwrap());
    }

    #[test]
    fn clone_removal_and_lease_acquisition_have_one_unlink_order() {
        for _ in 0..16 {
            let directory = tempfile::tempdir().unwrap();
            let store = BlobStore::new(directory.path());
            let hash = installed(&store);
            let remover = store.clone();
            let barrier = Arc::new(Barrier::new(2));
            let start = barrier.clone();
            let remove = std::thread::spawn(move || {
                start.wait();
                remover.remove_verified(hash, 1024).unwrap()
            });
            barrier.wait();
            let lease = store.lease_deletions();
            let present_at_acquisition = store.path_for(hash).exists();
            let removed = remove.join().unwrap();
            assert_eq!(removed, !present_at_acquisition);
            assert_eq!(store.path_for(hash).exists(), present_at_acquisition);
            drop(lease);
        }
    }
}
