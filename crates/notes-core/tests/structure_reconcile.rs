//! Structural operations reconcile every block's placement from the LWW intents. That walk is
//! global, so the cost of appending one block used to grow with the size of the workspace: on a
//! device with seventeen thousand blocks a single journal capture rewrote all of them inside one
//! transaction, on the single connection every other query waits behind, and the app stopped
//! answering for minutes. The reconcile still walks everything; it must not *write* what already
//! matches.

use notes_core::db::{self, BlockContent};
use notes_core::{BlockStyle, Connection, JournalDate};

async fn database() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let connection = db::open(directory.path().join("notes.db"))
        .await
        .expect("open database");
    (directory, connection)
}

async fn total_changes(connection: &Connection) -> i64 {
    connection
        .call(|database| Ok(database.total_changes() as i64))
        .await
        .expect("read total changes")
}

#[tokio::test]
async fn appending_a_block_does_not_rewrite_the_whole_workspace() {
    let (_directory, connection) = database().await;
    let note = db::create_note(&connection, Some("Structure".into()))
        .await
        .expect("create note")
        .into_created()
        .expect("a free title creates a note");

    let mut previous = note.initial_block.uuid;
    for index in 0..200 {
        previous = db::create_block(
            &connection,
            note.page.uuid,
            None,
            Some(previous),
            BlockStyle::Paragraph,
            format!("block {index}"),
        )
        .await
        .expect("create block")
        .uuid;
    }

    let before = total_changes(&connection).await;
    db::create_block(
        &connection,
        note.page.uuid,
        None,
        Some(previous),
        BlockStyle::Paragraph,
        "one more".into(),
    )
    .await
    .expect("create the block under test");
    let written = total_changes(&connection).await - before;

    // The new block, its structure intent, the operation bookkeeping and the undo entry are all
    // legitimate writes. The two hundred blocks that did not move are not: before the reconcile
    // learned to skip unchanged rows this was over two hundred.
    assert!(
        written < 60,
        "appending one block wrote {written} rows; the reconcile is rewriting untouched blocks"
    );
}

#[tokio::test]
async fn journal_capture_cost_does_not_grow_with_the_workspace() {
    let (_directory, connection) = database().await;
    let date = "2026-09-07".parse::<JournalDate>().expect("valid date");

    let mut costs = Vec::new();
    for round in 0..3 {
        for index in 0..50 {
            db::append_to_journal(
                &connection,
                date.clone(),
                BlockContent { markdown: format!("filler {round}-{index}") },
                BlockStyle::Paragraph,
            )
            .await
            .expect("append filler");
        }
        let before = total_changes(&connection).await;
        db::append_to_journal(
            &connection,
            date.clone(),
            BlockContent { markdown: format!("measured {round}") },
            BlockStyle::Paragraph,
        )
        .await
        .expect("append measured");
        costs.push(total_changes(&connection).await - before);
    }

    let (first, last) = (costs[0], costs[costs.len() - 1]);
    assert!(
        last <= first + 10,
        "capture cost grew with the journal: {costs:?}"
    );
}
