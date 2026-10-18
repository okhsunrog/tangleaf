//! Installation of downloaded immutable graphs and their visible ink metadata.

use super::{Committed, NotesStore, NotesTransaction};
use crate::ink::transfer::{Blobs, PreparedInkGraph};
use crate::{BlobHash, CoreError, CoreResult};

impl NotesStore {
    /// Validate binary data before taking the writer, then atomically install
    /// it and publish actual availability/draft changes after commit.
    pub async fn stage_ink_graph(
        &self,
        root_hash: BlobHash,
        blobs: Blobs,
    ) -> anyhow::Result<Committed<()>> {
        let prepared =
            tokio::task::spawn_blocking(move || PreparedInkGraph::prepare(root_hash, blobs))
                .await??;
        self.stage_prepared_ink_graph(prepared).await
    }

    async fn stage_prepared_ink_graph(
        &self,
        prepared: PreparedInkGraph,
    ) -> anyhow::Result<Committed<()>> {
        self.write(move |write| write.stage_ink_graph(prepared))
            .await
    }
}

impl NotesTransaction<'_, '_> {
    fn stage_ink_graph(&mut self, prepared: PreparedInkGraph) -> CoreResult<()> {
        let result = (|| {
            if self.failed || self.applied {
                return Err(CoreError::invalid(
                    "a store command must apply one ordered batch",
                ));
            }
            let (changes, wrote) = prepared.apply_in_transaction(self.sql)?;
            self.changes = changes;
            self.durable_changed |= wrote;
            self.applied = true;
            Ok(())
        })();
        self.failed |= result.is_err();
        result
    }
}

#[cfg(test)]
mod tests;
