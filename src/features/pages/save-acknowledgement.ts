import { useEffect, useMemo, useSyncExternalStore } from "react";

type VersionedSnapshot = Readonly<{ revision: string }>;
type Receipt<T> = Readonly<{ identity: string; baseRevisions: ReadonlySet<string>; persisted: T }>;

/** One clean acknowledgement while Query's deferred React props catch up. */
export class SaveAcknowledgement<T extends VersionedSnapshot> {
  #receipt: Receipt<T> | null = null;
  readonly #listeners = new Set<() => void>();

  constructor(readonly identity: string) {}

  getSnapshot = () => this.#receipt;
  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };
  acknowledge = (baseRevision: string, persisted: T) => {
    this.acknowledgeMany([baseRevision], persisted);
  };
  acknowledgeMany = (baseRevisions: Iterable<string>, persisted: T) => {
    const knownBases = new Set(baseRevisions);
    const bases =
      this.#receipt && knownBases.has(this.#receipt.persisted.revision)
        ? this.#receipt.baseRevisions
        : [];
    this.#receipt = {
      identity: this.identity,
      baseRevisions: new Set([...bases, ...knownBases]),
      persisted,
    };
    this.#listeners.forEach((listener) => listener());
  };
  release(receipt: Receipt<T>) {
    // An effect from an earlier render cannot retire a later acknowledgement.
    if (this.#receipt !== receipt) return;
    this.#receipt = null;
    this.#listeners.forEach((listener) => listener());
  }
}

/**
 * Clearing a draft is synchronous; Query's observer notification is deferred.
 * Keep it visible only over explicitly superseded local revisions. Consecutive
 * saves can both acknowledge before the parent's first notification is delivered.
 * A matching or different incoming revision wins, then retires this receipt.
 * This is separate from dirty overlays, save ownership and CodeMirror history.
 */
export function useSaveAcknowledgement<T extends VersionedSnapshot>(
  identity: string,
  incoming: T,
  shared?: SaveAcknowledgement<T>,
) {
  const handoff = useMemo(() => shared ?? new SaveAcknowledgement<T>(identity), [identity, shared]);
  const receipt = useSyncExternalStore(handoff.subscribe, handoff.getSnapshot, handoff.getSnapshot);
  const superseded =
    receipt?.identity === identity &&
    receipt.persisted.revision !== incoming.revision &&
    receipt.baseRevisions.has(incoming.revision);
  useEffect(() => {
    if (receipt && !superseded) handoff.release(receipt);
  }, [handoff, receipt, superseded]);
  const persisted = superseded ? receipt.persisted : incoming;
  return [persisted, handoff.acknowledge] as const;
}
