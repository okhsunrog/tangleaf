use super::*;

/// The accepted command keeps its publisher alive until commit. AppState stores
/// an observer-free handle so it does not retain its own AppHandle in a cycle.
pub(crate) fn observed_store(app: &AppHandle, state: &AppState) -> notes_core::store::NotesStore {
    state
        .store
        .clone()
        .with_commit_observer(commit_publisher(app))
}

pub(crate) fn commit_publisher(
    app: &AppHandle,
) -> impl Fn(&notes_core::store::CommitNotice) + Send + Sync + 'static {
    let app = app.clone();
    move |commit| {
        emit_domain(
            &app,
            DomainEvent::MutationCommitted {
                commit: commit.clone(),
            },
        );
    }
}
