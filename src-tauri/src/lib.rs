#[cfg(target_os = "android")]
mod android_tls;
mod app_lock;
mod attachment_protocol;
pub mod backup;
mod commands;
mod settings;
mod startup;
mod sync;
mod workspace;

use std::sync::{Arc, RwLock};
use tauri::Manager;
use tauri_specta::Event;

fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            commands::is_ready,
            commands::startup_status,
            commands::mobile_system_info,
            commands::input_capabilities,
            commands::create_handwritten_note,
            commands::load_handwriting_note,
            commands::handwriting_note_status,
            commands::preview_handwriting_version,
            commands::resolve_handwriting_conflict,
            commands::complete_all_handwriting,
            commands::set_handwriting_background,
            commands::handwriting_history,
            commands::save_handwriting_patch,
            commands::complete_handwriting_note,
            commands::set_system_bars_style,
            commands::set_immersive_mode,
            commands::set_display_profile,
            commands::request_full_refresh,
            commands::einkwise_status,
            commands::set_einkwise,
            commands::sync_status,
            commands::server_ai_status,
            commands::save_server_ai_settings,
            commands::save_server_ai_provider,
            commands::probe_server_ai_provider,
            commands::reindex_server_ai,
            commands::load_settings,
            settings::transfer::export_device_configuration,
            settings::transfer::preview_configuration_import,
            settings::transfer::apply_configuration_import,
            settings::transfer::cancel_configuration_import,
            commands::save_settings,
            commands::reset_settings,
            commands::retry_sync,
            commands::retry_rejected_changes,
            commands::restart_app,
            commands::history_status,
            commands::undo,
            commands::redo,
            commands::rename_page,
            commands::set_page_layout,
            commands::create_note,
            commands::ensure_journal,
            commands::get_journal,
            commands::list_journals,
            commands::append_to_journal,
            commands::get_page,
            commands::get_page_document,
            commands::get_page_render_snapshot,
            commands::replace_page_document,
            commands::get_block,
            commands::set_block_content,
            commands::set_block_style,
            commands::set_task_state,
            commands::split_block,
            commands::get_containing_page,
            commands::neighbors,
            commands::search_notes,
            commands::chat_stream,
            commands::cancel_chat,
            commands::list_pages,
            commands::delete_page,
            commands::find_backlinks,
            commands::graph_snapshot,
            commands::export_data,
            commands::import_data,
            commands::logseq_import_capability,
            commands::prepare_logseq_import,
            commands::logseq_import_diagnostics,
            commands::commit_logseq_import,
            commands::discard_logseq_import,
            commands::create_backup,
            commands::list_recovery_backups,
            commands::prepare_recovery_restore,
            commands::choose_recovery_restore,
            commands::discard_recovery_restore,
            commands::commit_recovery_restore,
            commands::attach_file,
            commands::list_attachments,
            commands::resolve_attachment_images,
            commands::open_attachment,
            commands::delete_attachment,
            commands::create_page,
            commands::list_block_children,
            commands::create_block,
            commands::indent_block,
            commands::outdent_block,
            commands::move_block_up,
            commands::move_block_down,
            commands::delete_block,
            commands::get_or_create_page_by_title,
            commands::get_page_by_title,
            commands::search_pages_by_title,
            commands::search_blocks_fts,
        ])
        .events(tauri_specta::collect_events![
            commands::DomainEventMessage,
            commands::StartupReadyEvent,
            commands::StartupErrorEvent
        ])
        // The UI contract intentionally uses JavaScript numbers. Every exposed 64-bit value is
        // bounded below Number.MAX_SAFE_INTEGER: Unix-second timestamps, attachment sizes capped
        // at 100 MiB, SQLite row counts, and server queue counters. HLCs and server sequences are
        // synchronization internals and never cross the Tauri command/event boundary.
        .dangerously_cast_bigints_to_number()
}

pub fn export_bindings(path: impl AsRef<std::path::Path>) -> Result<(), String> {
    specta_builder()
        .export(
            specta_typescript::Typescript::default()
                .header("/* eslint-disable */\n// Generated by tauri-specta. Do not edit.\n"),
            path,
        )
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    let specta_builder = specta_builder();

    #[cfg(all(debug_assertions, not(mobile)))]
    export_bindings("../src/lib/bindings.ts").expect("exporting TypeScript bindings");

    let invoke_handler = specta_builder.invoke_handler();
    let context = tauri::generate_context!();
    #[cfg(not(mobile))]
    let (context, main_window_config) = {
        let mut context = context;
        let config = context
            .config_mut()
            .app
            .windows
            .iter_mut()
            .find(|window| window.label == "main")
            .expect("main window configuration");
        let main_window_config = config.clone();
        config.create = false;
        (context, main_window_config)
    };
    let builder = tauri::Builder::default()
        .manage(commands::HandwritingStore::default())
        .manage(settings::transfer::PendingConfiguration::default())
        .register_asynchronous_uri_scheme_protocol(
            attachment_protocol::ATTACHMENT_PROTOCOL,
            attachment_protocol::protocol,
        )
        .plugin(tauri_plugin_opener::init());

    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    #[cfg(target_os = "android")]
    let builder = builder
        .plugin(tauri_plugin_android_fs::init())
        .plugin(tauri_plugin_mobile_system::init());

    #[cfg(target_os = "android")]
    let builder = builder.on_window_event(|window, event| {
        if matches!(
            event,
            tauri::WindowEvent::Suspended | tauri::WindowEvent::Resumed
        ) {
            window
                .app_handle()
                .state::<commands::HandwritingStore>()
                .set_background(matches!(event, tauri::WindowEvent::Suspended));
        }
        if matches!(event, tauri::WindowEvent::Resumed)
            && let Some(sync) = window.app_handle().try_state::<sync::SyncRuntime>()
        {
            sync.request_retry();
        }
    });

    // Debug builds and explicitly opted-in optimized diagnostic builds only.
    // Keep inspection on localhost; Android can use `adb forward tcp:9223 tcp:9223`.
    // The inspection config also enables the global Tauri API and its script CSP.
    #[cfg(all(
        any(debug_assertions, feature = "runtime-inspection"),
        not(target_os = "ios")
    ))]
    let builder = {
        tracing::info!(
            debug_assertions = cfg!(debug_assertions),
            "localhost runtime inspection enabled"
        );
        builder.plugin(
            tauri_plugin_mcp_bridge::Builder::new()
                .bind_address("127.0.0.1")
                .build(),
        )
    };

    let builder = builder
        .setup(move |app| {
            let (startup_lifecycle, registration) = startup::StartupLifecycle::register();
            app.manage(startup_lifecycle.clone());
            #[cfg(not(mobile))]
            settings::create_main_window(app, &main_window_config)?;
            specta_builder.mount_events(app);
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir().expect("resolving app data dir");
            std::fs::create_dir_all(&data_dir).expect("creating data dir");
            let shutdown = workspace::WorkspaceShutdown::default();
            app.manage(shutdown.clone());
            app.manage(backup::RecoveryRuntime::new(data_dir.clone(), shutdown));
            let blob_store = notes_blob::BlobStore::new(data_dir.clone());
            // v2: previews switched from lossless to lossy WebP; the old
            // lossless blobs would never be referenced again, so drop the dir.
            let image_cache = notes_blob::BlobStore::new(data_dir.join("image-cache-v2"));
            let stale_image_cache = data_dir.join("image-cache-v1");
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(stale_image_cache);
            });
            app.manage(commands::LogseqImportSessions::default());

            let startup = Arc::new(RwLock::new(commands::StartupStatus::Starting {
                message: "Loading device settings…".into(),
            }));
            handle.manage(commands::Startup {
                status: startup.clone(),
            });
            let sync_runtime = sync::SyncRuntime::disabled();
            let sync_status = sync_runtime.status.clone();
            let sync_retries = sync_runtime.subscribe_retries();
            app.manage(sync_runtime);

            let db_path = data_dir.join("notes.db");
            let configured_settings = match settings::runtime(&handle) {
                Ok(settings) => settings,
                Err(error) => {
                    finish_startup_error(
                        registration,
                        &startup_lifecycle,
                        &handle,
                        &startup,
                        error,
                    );
                    return Ok(());
                }
            };
            let sync_credentials = match configured_settings.sync_credentials() {
                Ok(credentials) => credentials,
                Err(error) => {
                    finish_startup_error(
                        registration,
                        &startup_lifecycle,
                        &handle,
                        &startup,
                        error,
                    );
                    return Ok(());
                }
            };
            let remote_ai = sync_credentials
                .as_ref()
                .map(|(server_url, token)| {
                    notes_sync::HttpTransport::new(server_url.clone(), token.clone())
                })
                .transpose();
            let remote_ai = match remote_ai {
                Ok(remote) => remote,
                Err(error) => {
                    finish_startup_error(
                        registration,
                        &startup_lifecycle,
                        &handle,
                        &startup,
                        error,
                    );
                    return Ok(());
                }
            };
            *startup.write().unwrap_or_else(|error| error.into_inner()) =
                commands::StartupStatus::Starting {
                    message: "Opening local notes database…".into(),
                };

            let failure_handle = handle.clone();
            let failure_status = startup.clone();
            registration.spawn(
                async move {
                    let application_lock = app_lock::ApplicationLock::acquire(&data_dir).await?;
                    startup_lifecycle.retain_application_lock(application_lock);
                    if startup_lifecycle.is_stopping() {
                        return Ok(());
                    }
                    let store = notes_core::store::NotesStore::open(&db_path).await?;
                    // Retain before any subsequent await or state installation;
                    // exit can still close this Store if initialization fails.
                    startup_lifecycle.retain_store(&store);
                    let conn = store.write_connection().clone();
                    handle.manage(commands::AppState {
                        store,
                        blob_store: blob_store.clone(),
                        image_cache,
                        preview_jobs: Arc::new(attachment_protocol::PreviewJobs::default()),
                        remote_ai,
                        chat_cancellations: Arc::new(std::sync::Mutex::new(
                            std::collections::HashMap::new(),
                        )),
                    });
                    if startup_lifecycle.is_stopping() {
                        return Ok(());
                    }
                    let ink = handle.state::<commands::HandwritingStore>().inner().clone();
                    if let Err(error) = ink.recover(&handle, &conn).await {
                        tracing::warn!(
                            ?error,
                            "Ink recovery deferred; existing notes remain available"
                        );
                    }
                    startup_lifecycle.while_starting(|| {
                        if let Some((server_url, token)) = sync_credentials {
                            sync::spawn_worker(
                                handle.clone(),
                                conn.clone(),
                                server_url,
                                token,
                                blob_store,
                                sync_status,
                                sync_retries,
                            )
                        }
                        *startup.write().unwrap_or_else(|e| e.into_inner()) =
                            commands::StartupStatus::Ready;
                        let _ = commands::StartupReadyEvent.emit(&handle);
                        tracing::info!(?db_path, "tangleaf ready");
                    });
                    Ok(())
                },
                move |message| report_startup_error(&failure_handle, &failure_status, message),
            );

            Ok(())
        })
        .invoke_handler(invoke_handler)
        .build(context)
        .expect("error while building tauri application");
    let finishing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    builder.run(move |app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            use std::sync::atomic::Ordering;
            if finished.load(Ordering::Relaxed) {
                return;
            }
            api.prevent_exit();
            if finishing.swap(true, Ordering::Relaxed) {
                return;
            }
            // Close Ready/sync publication synchronously, before the exit task
            // can be delayed by the executor or a long-running opener.
            let startup = app
                .try_state::<startup::StartupLifecycle>()
                .map(|state| state.inner().clone());
            if let Some(startup) = &startup {
                startup.stop();
            }
            let recovery = app
                .try_state::<backup::RecoveryRuntime>()
                .map(|state| state.inner().clone());
            let restore_completion = recovery
                .as_ref()
                .and_then(backup::RecoveryRuntime::begin_exit);
            let shutdown = app
                .try_state::<workspace::WorkspaceShutdown>()
                .map(|state| state.inner().clone());
            let app = app.clone();
            let finished = finished.clone();
            tauri::async_runtime::spawn(async move {
                backup::RecoveryRuntime::wait_before_exit(restore_completion).await;
                if let Some(shutdown) = shutdown {
                    shutdown.begin(app.clone());
                    match shutdown.wait().await {
                        Ok(_) => {
                            // A closed token retained by recovery keeps its
                            // independent lock through publication and exit.
                            if let Some(startup) = startup {
                                startup.release_application_lock();
                            }
                        }
                        Err(error) => tracing::error!(%error, "Workspace close failed during exit"),
                    }
                }
                finished.store(true, Ordering::Relaxed);
                app.exit(code.unwrap_or(0));
            });
        }
    });
}

fn report_startup_error(
    handle: &tauri::AppHandle,
    startup: &Arc<RwLock<commands::StartupStatus>>,
    message: String,
) {
    tracing::error!(%message, "tangleaf startup failed");
    *startup.write().unwrap_or_else(|error| error.into_inner()) = commands::StartupStatus::Error {
        message: message.clone(),
    };
    let _ = commands::StartupErrorEvent { message }.emit(handle);
}

fn finish_startup_error(
    registration: startup::Registration,
    lifecycle: &startup::StartupLifecycle,
    handle: &tauri::AppHandle,
    startup: &Arc<RwLock<commands::StartupStatus>>,
    error: anyhow::Error,
) {
    let message = format!("{error:#}");
    lifecycle.while_starting(|| report_startup_error(handle, startup, message.clone()));
    registration.finish(Err(message));
}

#[cfg(not(target_os = "android"))]
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_filter())
        .init();
}

fn tracing_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
}

#[cfg(target_os = "android")]
fn init_tracing() {
    use tracing_logcat::{LogcatMakeWriter, LogcatTag};

    let writer = LogcatMakeWriter::new(LogcatTag::Fixed("tangleaf".into()))
        .expect("creating Android logcat writer");
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(true)
        .with_env_filter(tracing_filter())
        .with_writer(writer)
        .init();
}
