use super::*;

#[tauri::command]
#[specta::specta]
pub async fn list_block_children(
    state: State<'_, AppState>,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
) -> CommandResult<Vec<db::Block>> {
    db::list_block_children(state.store.read_connection(), page_uuid, parent_uuid)
        .await
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn get_block(
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
) -> CommandResult<Option<db::Block>> {
    db::get_block(state.store.read_connection(), uuid)
        .await
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn create_block(
    app: AppHandle,
    state: State<'_, AppState>,
    page_uuid: uuid::Uuid,
    parent_uuid: Option<uuid::Uuid>,
    after_uuid: Option<uuid::Uuid>,
    style: BlockStyle,
    markdown: String,
) -> CommandResult<db::Block> {
    let committed = observed_store(&app, &state)
        .create_block(page_uuid, parent_uuid, after_uuid, style, markdown)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn set_block_content(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
    content: db::BlockContent,
    expected_revision: ContentRevision,
) -> CommandResult<db::Block> {
    let committed = observed_store(&app, &state)
        .set_block_content_if_revision(uuid, content, expected_revision)
        .await
        .map_err(err)?;
    Ok(committed.value.0)
}

#[tauri::command]
#[specta::specta]
pub async fn set_block_style(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
    style: BlockStyle,
) -> CommandResult<db::Block> {
    let committed = observed_store(&app, &state)
        .set_block_style(uuid, style)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn set_task_state(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
    task_state: TaskState,
) -> CommandResult<db::Block> {
    let committed = observed_store(&app, &state)
        .set_task_state(uuid, task_state)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn split_block(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
    parts: Vec<db::BlockContent>,
    expected_revision: ContentRevision,
) -> CommandResult<Vec<db::Block>> {
    let committed = observed_store(&app, &state)
        .split_block(uuid, parts, expected_revision)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

async fn change_indent(
    app: &AppHandle,
    state: &AppState,
    uuid: uuid::Uuid,
    indent: bool,
) -> CommandResult<db::Block> {
    let store = observed_store(app, state);
    let committed = if indent {
        store.indent_block(uuid).await
    } else {
        store.outdent_block(uuid).await
    }
    .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn indent_block(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
) -> CommandResult<db::Block> {
    change_indent(&app, &state, uuid, true).await
}

#[tauri::command]
#[specta::specta]
pub async fn outdent_block(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
) -> CommandResult<db::Block> {
    change_indent(&app, &state, uuid, false).await
}

async fn move_direction(
    app: &AppHandle,
    state: &AppState,
    uuid: uuid::Uuid,
    direction: ReorderDirection,
) -> CommandResult<db::Block> {
    let committed = observed_store(app, state)
        .move_block_in_direction(uuid, direction)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn move_block_up(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
) -> CommandResult<db::Block> {
    move_direction(&app, &state, uuid, ReorderDirection::Up).await
}

#[tauri::command]
#[specta::specta]
pub async fn move_block_down(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
) -> CommandResult<db::Block> {
    move_direction(&app, &state, uuid, ReorderDirection::Down).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_block(
    app: AppHandle,
    state: State<'_, AppState>,
    uuid: uuid::Uuid,
) -> CommandResult<bool> {
    let committed = observed_store(&app, &state)
        .delete_block(uuid)
        .await
        .map_err(err)?;
    Ok(committed.value)
}

#[tauri::command]
#[specta::specta]
pub async fn history_status(state: State<'_, AppState>) -> CommandResult<db::HistoryStatus> {
    db::history_status(state.store.read_connection())
        .await
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn undo(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<db::HistoryMoveResult> {
    Ok(observed_store(&app, &state)
        .undo()
        .await
        .map_err(err)?
        .value)
}

#[tauri::command]
#[specta::specta]
pub async fn redo(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<db::HistoryMoveResult> {
    Ok(observed_store(&app, &state)
        .redo()
        .await
        .map_err(err)?
        .value)
}

#[tauri::command]
#[specta::specta]
pub async fn search_pages_by_title(
    state: State<'_, AppState>,
    query: String,
    limit: u32,
) -> CommandResult<Vec<db::Page>> {
    db::search_pages_by_title(state.store.read_connection(), query, limit)
        .await
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn search_blocks_fts(
    state: State<'_, AppState>,
    query: String,
    limit: u32,
    token_mode: notes_core::SearchTokenMode,
) -> CommandResult<Vec<db::Block>> {
    db::search_blocks_fts(state.store.read_connection(), query, limit, token_mode)
        .await
        .map_err(err)
}
