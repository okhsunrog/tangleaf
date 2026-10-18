use super::*;

#[tokio::test]
async fn migration_failure_drains_and_closes_the_unreturned_connection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.db");
    let connection = Connection::open(&path).await.unwrap();
    connection
        .call(|sql| {
            sql.pragma_update(None, "user_version", i32::MAX)?;
            sql.execute_batch(
                "CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES('kept')",
            )
        })
        .await
        .unwrap();
    let retained = connection.clone();
    assert!(initialize_connection(connection).await.is_err());
    assert!(retained.call(|_| Ok(())).await.is_err());
    retained.close().await.unwrap();
    let reopened = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = reopened
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, i64::from(i32::MAX));
    assert_eq!(
        reopened
            .query_row("SELECT value FROM preserved", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "kept"
    );
}

#[tokio::test]
async fn projection_failure_also_drains_the_initialized_worker() {
    let directory = tempfile::tempdir().unwrap();
    let connection = open(directory.path().join("notes.db")).await.unwrap();
    connection
        .call(|sql| {
            sql.execute("UPDATE store_state SET structure_projection_version=2", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let retained = connection.clone();
    let error = initialize_connection(connection).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("structural projection version 2")
    );
    assert!(retained.call(|_| Ok(())).await.is_err());
    retained.close().await.unwrap();
}
