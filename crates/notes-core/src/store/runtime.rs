use super::NotesStore;
use crate::{Connection, db};
use anyhow::{Result, anyhow};
use std::path::Path;

impl NotesStore {
    /// Initialize/migrate the writer first, then open one independent read-only
    /// worker against the same file. No read worker can observe a partial schema.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        let writer = db::open(&path).await?;
        let reader = Connection::open_read_only(path).await;
        Self::finish_open(writer, reader).await
    }

    async fn finish_open(writer: Connection, reader: Result<Connection>) -> Result<Self> {
        match reader {
            Ok(reader) => {
                let mut store = Self::new(writer);
                store.reader = Some(reader);
                Ok(store)
            }
            Err(error) => {
                // Do not return an initialization failure while leaving the
                // already-created writer running in the background.
                match writer.close().await {
                    Ok(()) => Err(error.context("opening SQLite read worker")),
                    Err(close) => Err(error.context(format!(
                        "opening SQLite read worker; writer cleanup also failed: {close:#}"
                    ))),
                }
            }
        }
    }

    /// The independent read worker for `open`; compatibility stores created
    /// with `new` share the supplied connection, including in-memory databases.
    pub fn read_connection(&self) -> &Connection {
        self.reader.as_ref().unwrap_or(&self.connection)
    }

    /// The initialized writer used by all Store mutations.
    pub fn write_connection(&self) -> &Connection {
        &self.connection
    }

    /// Stop both workers promptly and drain/join both, even if either fails.
    /// Once polled, cancellation leaves both independent shutdowns running.
    /// Clones retain handles but cannot submit new work after close starts.
    pub async fn close(&self) -> Result<()> {
        let close_reader = async {
            match &self.reader {
                Some(reader) => reader.close().await,
                None => Ok(()),
            }
        };
        // join polls both closes before yielding. Awaiting them sequentially
        // would leave the second worker accepting work if the waiter canceled.
        let (writer, reader) = tokio::join!(self.connection.close(), close_reader);
        match (writer, reader) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(writer), Ok(())) => Err(writer.context("closing SQLite write worker")),
            (Ok(()), Err(reader)) => Err(reader.context("closing SQLite read worker")),
            (Err(writer), Err(reader)) => Err(anyhow!(
                "closing SQLite workers failed: writer: {writer:#}; reader: {reader:#}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reader_open_failure_closes_and_joins_the_initialized_writer() {
        let directory = tempfile::tempdir().unwrap();
        let writer = db::open(directory.path().join("notes.db")).await.unwrap();
        let retained = writer.clone();
        let result = NotesStore::finish_open(writer, Err(anyhow!("reader open failed"))).await;
        let error = result.err().unwrap();
        assert!(format!("{error:#}").contains("reader open failed"));
        assert!(retained.call(|_| Ok(())).await.is_err());
        retained.close().await.unwrap();
    }
}
