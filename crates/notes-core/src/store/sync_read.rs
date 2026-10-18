//! Consistent sync bootstrap reads on the Store's independent reader.

use super::NotesStore;
use crate::{CoreResult, Op, SyncSnapshot, operation};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncQueueStatus {
    pub cursor: u64,
    pub pending_operations: u32,
    pub quarantined_operations: u32,
    pub quarantine_reason: Option<String>,
}

impl NotesStore {
    /// Queue metadata without decoding or cloning any persisted envelopes.
    pub async fn sync_queue_status(&self) -> anyhow::Result<SyncQueueStatus> {
        self.read_connection()
            .call_domain(|database| -> anyhow::Result<_> {
                let transaction = database.transaction()?;
                let cursor = operation::read_sync_cursor(&transaction)?;
                let (pending, quarantined, reason) = transaction.query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM sync_outbox WHERE quarantined_at IS NULL),
                        (SELECT COUNT(*) FROM sync_outbox WHERE quarantined_at IS NOT NULL),
                        (SELECT COALESCE(quarantine_reason, '') FROM sync_outbox
                         WHERE quarantined_at IS NOT NULL ORDER BY rowid LIMIT 1)",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )?;
                transaction.commit()?;
                Ok(SyncQueueStatus {
                    cursor,
                    pending_operations: pending.try_into().unwrap_or(u32::MAX),
                    quarantined_operations: quarantined.try_into().unwrap_or(u32::MAX),
                    quarantine_reason: reason,
                })
            })
            .await
    }

    /// Capture the content baseline and its sendable outbox in one SQLite read
    /// snapshot. Remote/native changes need not modify the local outbox, so
    /// comparing outbox reads around an independently exported snapshot cannot
    /// establish this guarantee.
    pub async fn capture_sync_baseline(&self, seq: u64) -> anyhow::Result<(SyncSnapshot, Vec<Op>)> {
        self.read_connection()
            .call_domain(move |database| -> CoreResult<_> {
                let transaction = database.transaction()?;
                let baseline = read_sync_baseline(&transaction, seq)?;
                transaction.commit()?;
                Ok(baseline)
            })
            .await
    }
}

fn read_sync_baseline(
    transaction: &rusqlite::Transaction<'_>,
    seq: u64,
) -> CoreResult<(SyncSnapshot, Vec<Op>)> {
    Ok((
        operation::export_sync_snapshot_in_transaction(transaction, seq)?,
        operation::read_pending_outbox(transaction, u32::MAX)?,
    ))
}

#[cfg(test)]
mod tests;
