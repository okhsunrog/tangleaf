//! Binary ownership follows the durable outbox row, including quarantine.
use super::{CommandError, CommandResult, storage};
use crate::{Op, OpKind, decode_persisted_envelope};
use ink_format::Id;
use notes_blob::BlobHash;
use rusqlite::{Connection, params};

/// Called in the same transaction immediately after the envelope is queued.
/// Native publication persists its immutable root before Apply, but assigns
/// ink_versions.root_id afterwards, so resolve through the record hash index.
pub(crate) fn enqueue(conn: &Connection, operation: &Op) -> CommandResult<()> {
    let OpKind::InkPublish(publication) = &operation.kind else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO ink_outbox_roots(op_id,root_hash,root_id)
         VALUES(?1,?2,(SELECT id FROM ink_records WHERE hash=?2))",
        params![operation.op_id, publication.root_hash.as_bytes().as_slice()],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO ink_sealed_chunks
         SELECT refs.object_id FROM ink_root_refs refs
         JOIN ink_outbox_roots held ON held.root_id=refs.root_id
         WHERE held.op_id=?1 AND refs.kind=1",
        [operation.op_id],
    )?;
    Ok(())
}

/// A graph installed after its publication metadata fulfills the same owner.
/// This is an indexed update; gesture GC never parses outbox JSON.
pub(in crate::ink) fn attach_root(conn: &Connection, root: Id) -> CommandResult<()> {
    let filled = conn.execute(
        "UPDATE ink_outbox_roots SET root_id=?1
         WHERE root_hash=(SELECT hash FROM ink_records WHERE id=?1) AND root_id IS NULL",
        [root.as_slice()],
    )?;
    if filled != 0 {
        conn.execute(
            "INSERT OR IGNORE INTO ink_sealed_chunks
             SELECT object_id FROM ink_root_refs WHERE root_id=?1 AND kind=1",
            [root.as_slice()],
        )?;
    }
    Ok(())
}

/// Return whether the outbox changed and whether an ink owner was released.
/// The caller collects once after its complete acknowledgement batch.
pub(crate) fn remove_operation(
    conn: &Connection,
    op_id: uuid::Uuid,
) -> CommandResult<(bool, bool)> {
    let released = conn.execute("DELETE FROM ink_outbox_roots WHERE op_id=?1", [op_id])? != 0;
    let removed = conn.execute("DELETE FROM sync_outbox WHERE op_id=?1", [op_id])? != 0;
    Ok((removed, released))
}

pub(crate) fn collect(conn: &Connection) -> CommandResult<()> {
    storage::history::collect(conn)
}

/// Runs as the V011 migration hook, in the DDL transaction. Missing bytes stay
/// explicitly unavailable; malformed envelopes cannot silently evade ownership.
pub(crate) fn backfill(conn: &Connection) -> CommandResult<()> {
    let mut statement = conn.prepare("SELECT op_id,envelope FROM sync_outbox ORDER BY rowid")?;
    let pending = statement.query_map([], |row| {
        Ok((row.get::<_, uuid::Uuid>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in pending {
        let (op_id, envelope) = row?;
        let operation: Op = decode_persisted_envelope(&envelope).map_err(|error| {
            CommandError::invalid(format!("invalid outbox envelope {op_id}: {error}"))
        })?;
        if operation.op_id != op_id {
            return Err(CommandError::invalid(format!(
                "outbox envelope {op_id} contains a different operation ID"
            )));
        }
        enqueue(conn, &operation)?;
    }
    Ok(())
}

pub(in crate::ink) fn root_for_hash(
    conn: &Connection,
    root_hash: BlobHash,
) -> rusqlite::Result<Option<Vec<u8>>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT root_id FROM ink_outbox_roots WHERE root_hash=?1 AND root_id IS NOT NULL
         UNION ALL
         SELECT root_id FROM ink_versions WHERE root_hash=?1 AND root_id IS NOT NULL
         LIMIT 1",
        [root_hash.as_bytes().as_slice()],
        |row| row.get(0),
    )
    .optional()
}
