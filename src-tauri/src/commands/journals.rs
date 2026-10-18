use super::*;

#[tauri::command]
#[specta::specta]
pub async fn ensure_journal(
    app: AppHandle,
    state: State<'_, AppState>,
    date: notes_core::JournalDate,
) -> CommandResult<db::Page> {
    let committed = observed_store(&app, &state)
        .ensure_journal(date)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn get_journal(
    state: State<'_, AppState>,
    date: notes_core::JournalDate,
) -> CommandResult<Option<db::Page>> {
    db::get_journal(state.store.read_connection(), date)
        .await
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn list_journals(
    state: State<'_, AppState>,
    before_date: Option<notes_core::JournalDate>,
    limit: Option<db::JournalListLimit>,
) -> CommandResult<Vec<db::Page>> {
    db::list_journals(
        state.store.read_connection(),
        before_date,
        limit.unwrap_or_default(),
    )
    .await
    .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn append_to_journal(
    app: AppHandle,
    state: State<'_, AppState>,
    date: notes_core::JournalDate,
    content: db::BlockContent,
    style: BlockStyle,
) -> CommandResult<db::Block> {
    let committed = observed_store(&app, &state)
        .append_to_journal(date, content, style)
        .await
        .map_err(err)?;
    Ok(committed.value)
}
