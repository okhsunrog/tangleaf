use super::*;
use crate::backup::{
    RecoveryBackupEntry, RecoveryRestorePreview, RecoveryRestoreResult, RecoveryRuntime,
};

#[tauri::command]
#[specta::specta]
pub async fn list_recovery_backups(
    recovery: State<'_, RecoveryRuntime>,
) -> CommandResult<Vec<RecoveryBackupEntry>> {
    recovery.list_backups().await.map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn prepare_recovery_restore(
    source_path: String,
    state: State<'_, AppState>,
    recovery: State<'_, RecoveryRuntime>,
) -> CommandResult<RecoveryRestorePreview> {
    recovery
        .prepare(source_path.into(), &state.store)
        .await
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn choose_recovery_restore(
    app: AppHandle,
    state: State<'_, AppState>,
    recovery: State<'_, RecoveryRuntime>,
) -> CommandResult<Option<RecoveryRestorePreview>> {
    #[cfg(not(target_os = "android"))]
    {
        let Some(folder) = app
            .dialog()
            .file()
            .set_title("Choose a .recovery backup folder")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        recovery
            .prepare(folder.into_path().map_err(err)?, &state.store)
            .await
            .map(Some)
            .map_err(err)
    }
    #[cfg(target_os = "android")]
    {
        let _ = (app, state, recovery);
        Err(CommandError::new(
            CommandErrorCode::Unavailable,
            "Choose a local app backup from the list on Android",
        ))
    }
}

#[tauri::command]
#[specta::specta]
pub fn discard_recovery_restore(
    token: uuid::Uuid,
    recovery: State<'_, RecoveryRuntime>,
) -> CommandResult<()> {
    recovery.discard(token).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn commit_recovery_restore(
    token: uuid::Uuid,
    app: AppHandle,
    recovery: State<'_, RecoveryRuntime>,
) -> CommandResult<RecoveryRestoreResult> {
    recovery.commit(token, &app).await.map_err(err)
}
