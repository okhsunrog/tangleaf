//! Advisory lifetime ownership for cooperating application processes.
//!
//! Keep the same lock file inode across launches. External SQLite clients do
//! not participate in this protocol and must be stopped for physical restore.

use anyhow::{Context, Result, bail, ensure};
use std::fs::File;
use std::path::Path;
use std::time::Duration;

const LOCK_FILE: &str = ".tangleaf-workspace.lock";
const RESTART_RETRY: Duration = Duration::from_secs(3);
const RETRY_INTERVAL: Duration = Duration::from_millis(25);

pub(crate) struct ApplicationLock {
    _file: File,
}

impl ApplicationLock {
    pub(crate) async fn acquire(directory: &Path) -> Result<Self> {
        let path = directory.join(LOCK_FILE);
        let file = tokio::task::spawn_blocking(move || open_lock_file(&path)).await??;
        Self::acquire_opened(file, RESTART_RETRY).await
    }

    async fn acquire_opened(file: File, retry: Duration) -> Result<Self> {
        let deadline = tokio::time::Instant::now() + retry;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(std::fs::TryLockError::WouldBlock) => {
                    // A restart child can briefly overlap its exiting parent.
                    // This wait is bounded; a second live app never opens SQL.
                    if tokio::time::Instant::now() >= deadline {
                        bail!("This workspace is already open in another tangleaf process");
                    }
                    tokio::time::sleep_until(
                        (tokio::time::Instant::now() + RETRY_INTERVAL).min(deadline),
                    )
                    .await;
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(error).context("locking the workspace application file");
                }
            }
        }
    }
}

fn open_lock_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    let file = File::from(rustix::fs::open(
        path,
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )?);
    #[cfg(not(unix))]
    let file = {
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "workspace lock is a symlink"
            );
        }
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?
    };
    ensure!(
        file.metadata()?.is_file(),
        "workspace lock is not a regular file"
    );
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_restart_child_can_wait_for_the_previous_owner_but_not_forever() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK_FILE);
        let first = ApplicationLock::acquire_opened(open_lock_file(&path).unwrap(), Duration::ZERO)
            .await
            .unwrap();
        let second_file = open_lock_file(&path).unwrap();
        let waiting = tokio::spawn(ApplicationLock::acquire_opened(
            second_file,
            Duration::from_millis(100),
        ));
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        tokio::time::advance(Duration::from_millis(25)).await;
        drop(first);
        tokio::time::advance(Duration::from_millis(25)).await;
        let second = waiting.await.unwrap().unwrap();
        let started = tokio::time::Instant::now();
        let error = ApplicationLock::acquire_opened(
            open_lock_file(&path).unwrap(),
            Duration::from_millis(100),
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("already open"));
        assert_eq!(started.elapsed(), Duration::from_millis(100));
        drop(second);
    }

    #[test]
    fn independently_opened_processes_observe_the_same_lifetime_lock() {
        let directory = tempfile::tempdir().unwrap();
        let file = open_lock_file(&directory.path().join(LOCK_FILE)).unwrap();
        file.try_lock().unwrap();
        let owner = ApplicationLock { _file: file };
        let probe = |expected| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "app_lock::tests::child_process_lock_probe",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env("TANGLEAF_TEST_LOCK_DIRECTORY", directory.path())
                .env("TANGLEAF_TEST_LOCK_EXPECTED", expected)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        probe("busy");
        drop(owner);
        probe("free");
        assert!(directory.path().join(LOCK_FILE).is_file());
    }

    #[tokio::test]
    async fn the_startup_owner_holds_the_lock_through_store_close_until_explicit_release() {
        let directory = tempfile::tempdir().unwrap();
        let lock = ApplicationLock::acquire(directory.path()).await.unwrap();
        let (startup, registration) = crate::startup::StartupLifecycle::register();
        startup.retain_application_lock(lock);
        let store = notes_core::store::NotesStore::open(directory.path().join("notes.db"))
            .await
            .unwrap();
        startup.retain_store(&store);
        registration.finish(Ok(()));
        startup.stop();
        startup.wait().await.unwrap();
        let retained = startup.clone();
        drop(startup);
        store.close().await.unwrap();
        let contender = open_lock_file(&directory.path().join(LOCK_FILE)).unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        retained.release_application_lock();
        contender.try_lock().unwrap();
    }

    #[test]
    #[ignore = "run by the parent process test with an isolated temporary workspace"]
    fn child_process_lock_probe() {
        let directory = std::env::var_os("TANGLEAF_TEST_LOCK_DIRECTORY").unwrap();
        let file = open_lock_file(&Path::new(&directory).join(LOCK_FILE)).unwrap();
        let result = file.try_lock();
        match std::env::var("TANGLEAF_TEST_LOCK_EXPECTED").as_deref() {
            Ok("busy") => assert!(matches!(result, Err(std::fs::TryLockError::WouldBlock))),
            Ok("free") => result.unwrap(),
            _ => panic!("child probe requires an expected lock state"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_non_regular_lock_paths_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCK_FILE);
        let target = directory.path().join("target");
        std::fs::write(&target, b"untouched").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(open_lock_file(&path).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(open_lock_file(&path).is_err());
    }
}
