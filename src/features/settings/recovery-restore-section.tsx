import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { FolderOpen, RotateCcw } from "lucide-react";
import { useConfirmation } from "@/app/confirmation";
import { flushWorkspaceDrafts } from "@/app/lifecycle-flush";
import { isAndroid } from "@/app/sheet-mode";
import { BusyIndicator } from "@/components/ui/busy-indicator";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { usePageSessionRegistry } from "@/features/pages/page-session";
import {
  chooseRecoveryRestore,
  commitRecoveryRestore,
  discardRecoveryRestore,
  listRecoveryBackups,
  prepareRecoveryRestore,
  restartApp,
  type RecoveryRestorePreview,
  type RecoveryRestoreResult,
} from "@/lib/api";
import { registerBackOverlay } from "@/lib/back-overlays";
import { queryKeys } from "@/lib/query";
import { SettingsSection } from "./settings-controls";
import { SettingsSelect } from "./settings-select";

type Props = {
  disabled: boolean;
  dataAvailable: boolean;
  onBusyChange: (busy: boolean) => void;
};

type Outcome = RecoveryRestoreResult | { status: "unknown"; error: string; restartRequired: true };
const created = (seconds: number) => new Date(seconds * 1000).toLocaleString();

export function RecoveryRestoreSection({ disabled, dataAvailable, onBusyChange }: Props) {
  const confirm = useConfirmation();
  const sessions = usePageSessionRegistry();
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<RecoveryRestorePreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [result, setResult] = useState<Outcome | null>(null);
  const [error, setError] = useState("");
  const mounted = useRef(true);
  const backups = useQuery({
    queryKey: queryKeys.recoveryBackups,
    queryFn: listRecoveryBackups,
    enabled: dataAvailable,
  });
  const locked = restoring || Boolean(result?.restartRequired);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    onBusyChange(loading || preview !== null || locked);
  }, [loading, preview, locked, onBusyChange]);
  const token = preview?.token;
  useEffect(
    () => () => {
      if (token) void discardRecoveryRestore(token).catch(() => undefined);
    },
    [token],
  );
  useEffect(() => {
    // A drained Store cannot resume. Android Back must not reveal the old workspace.
    if (locked) return registerBackOverlay(() => {});
  }, [locked]);

  async function prepare(path?: string) {
    setLoading(true);
    setError("");
    setResult(null);
    try {
      const prepared = path ? await prepareRecoveryRestore(path) : await chooseRecoveryRestore();
      if (!mounted.current) {
        if (prepared) await discardRecoveryRestore(prepared.token);
        return;
      }
      setPreview(prepared);
    } catch (reason) {
      if (mounted.current) setError(String(reason));
    } finally {
      if (mounted.current) setLoading(false);
    }
  }

  async function restore() {
    if (!preview || loading) return;
    if (
      !(await confirm({
        title: "Restore this backup?",
        description: `Replace local notes, history and unsynced changes with the backup from ${created(preview.createdAt)}. A recovery backup of the current workspace is created first. You will need to restart the app.`,
        confirmLabel: "Restore backup",
        destructive: true,
      }))
    )
      return;
    setLoading(true);
    setError("");
    try {
      await flushWorkspaceDrafts(sessions);
    } catch (reason) {
      setError(String(reason));
      setLoading(false);
      return;
    }
    setRestoring(true);
    try {
      setResult(await commitRecoveryRestore(preview.token));
    } catch (reason) {
      // A transport failure cannot establish whether the owned coordinator
      // accepted the request. Do not present the closed workspace as usable.
      setResult({ status: "unknown", error: String(reason), restartRequired: true });
    } finally {
      setPreview(null);
      setRestoring(false);
      setLoading(false);
    }
  }

  async function restart() {
    setRestarting(true);
    setError("");
    try {
      await restartApp();
    } catch (reason) {
      setError(String(reason));
      setRestarting(false);
    }
  }

  const path = selected ?? backups.data?.[0]?.path;
  const blocked = disabled || !dataAvailable || loading || preview !== null || locked;
  return (
    <SettingsSection
      title="Restore a recovery backup"
      description="Restore notes, attachments, local history and pending sync changes from a .recovery folder. Device settings stay as they are."
    >
      <div className="flex flex-wrap items-center gap-2">
        <div className="min-w-0 flex-1 basis-56">
          <SettingsSelect
            label="Local recovery backup"
            value={path ?? null}
            options={(backups.data ?? []).map((backup) => ({
              value: backup.path,
              label: `${created(backup.createdAt)} · ${backup.blobCount} files`,
            }))}
            placeholder={backups.isPending ? "Loading backups…" : "No local backups"}
            disabled={blocked || !backups.data?.length}
            onValueChange={setSelected}
          />
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={blocked || !path}
          onClick={() => void prepare(path)}
        >
          Review backup
        </Button>
        {!isAndroid() && (
          <Button type="button" variant="outline" disabled={blocked} onClick={() => void prepare()}>
            <FolderOpen className="size-4" /> Choose folder
          </Button>
        )}
        <Button
          type="button"
          variant="ghost"
          disabled={blocked}
          onClick={() => void backups.refetch()}
        >
          Refresh list
        </Button>
      </div>
      {backups.error && (
        <p role="alert" className="text-sm text-destructive">
          Could not list backups: {String(backups.error)}
        </p>
      )}
      {loading && !restoring && <BusyIndicator label="Checking backup…" />}
      {preview && (
        <div
          aria-label="Recovery backup preview"
          className="space-y-3 rounded-2xl border border-border/60 surface-base p-4"
        >
          <h3 className="font-semibold">Backup from {created(preview.createdAt)}</h3>
          <p className="text-sm text-muted-foreground">
            {preview.pageCount.toLocaleString()} pages · {preview.blockCount.toLocaleString()}{" "}
            blocks · {preview.blobCount.toLocaleString()} files
          </p>
          {preview.workspaceDiffers && (
            <p className="text-sm text-destructive">
              This backup belongs to another workspace. Your current server connection settings will
              be kept; check them before connecting after restore.
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={loading}
              onClick={() => setPreview(null)}
            >
              Cancel restore
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={loading}
              onClick={() => void restore()}
            >
              Restore backup
            </Button>
          </div>
        </div>
      )}
      {!locked && error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!locked && result?.status === "unchanged" && (
        <p role="status" className="text-sm text-muted-foreground">
          {result.error ?? "The current workspace was kept."}
        </p>
      )}
      <Dialog open={locked} disablePointerDismissal onOpenChange={(_, details) => details.cancel()}>
        <DialogContent
          showCloseButton={false}
          className="max-h-[calc(100dvh-2rem)] overflow-y-auto rounded-2xl"
          onKeyDown={(event) => event.stopPropagation()}
        >
          <DialogHeader>
            <DialogTitle>
              {restoring
                ? "Restoring backup…"
                : result?.status === "restored"
                  ? "Backup restored"
                  : result?.status === "published_durability_uncertain"
                    ? "Backup replaced; durability could not be confirmed"
                    : result?.status === "unknown"
                      ? "Restore result could not be confirmed"
                      : "Restore stopped"}
            </DialogTitle>
            <DialogDescription>
              {restoring
                ? "Finishing accepted writes and preparing a recovery copy before replacement. Keep the app open."
                : "Restart the app to reopen your workspace."}
            </DialogDescription>
          </DialogHeader>
          {restoring && <BusyIndicator label="Restoring" />}
          {result?.error && (
            <p role="alert" className="text-sm break-words text-destructive">
              {result.error}
            </p>
          )}
          {result && "warning" in result && result.warning && (
            <p className="text-sm text-muted-foreground">{result.warning}</p>
          )}
          {result && "beforeRestoreBackupPath" in result && result.beforeRestoreBackupPath && (
            <p className="text-sm text-muted-foreground">
              Before-restore backup:{" "}
              <span className="block break-all">{result.beforeRestoreBackupPath}</span>
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm break-words text-destructive">
              {error}
            </p>
          )}
          {!restoring && (
            <DialogFooter>
              <Button type="button" disabled={restarting} onClick={() => void restart()}>
                <RotateCcw className="size-4" />
                {restarting ? "Restarting…" : "Restart app"}
              </Button>
            </DialogFooter>
          )}
        </DialogContent>
      </Dialog>
    </SettingsSection>
  );
}
