import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getEinkWiseStatus, setEinkWise } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { ToggleField } from "./settings-controls";

/**
 * Whether BOOX's EinkWise service handles Tangleaf. Only a BOOX with the service reports it
 * available; everywhere else the field is not shown. The firmware enforces a change after the next
 * reboot, so the field says so while the stored choice and the enforced one differ.
 */
export function EinkWiseField() {
  const queryClient = useQueryClient();
  const status = useQuery({ queryKey: queryKeys.einkWise, queryFn: getEinkWiseStatus });
  const change = useMutation({
    mutationFn: (enabled: boolean) => setEinkWise(enabled),
    onSuccess: (next) => queryClient.setQueryData(queryKeys.einkWise, next),
  });
  const state = status.data;
  if (!state?.available) return null;

  return (
    <div className="space-y-2">
      <ToggleField
        label="EinkWise"
        description="On, the BOOX firmware applies the DPI, contrast and refresh settings chosen for Tangleaf in EinkWise, and adds a full refresh after a set number of taps. Off, Tangleaf runs the way the stock Notes app does: no counted refreshes, and the EinkWise DPI and settings no longer apply."
        checked={state.enabled ?? true}
        disabled={change.isPending}
        onChange={(enabled) => change.mutate(enabled)}
      />
      {state.enabled !== state.active && (
        <p role="status" className="text-sm text-muted-foreground">
          Takes effect after the device restarts.
        </p>
      )}
      {change.isError && (
        <p role="alert" className="text-sm text-destructive">
          Could not change EinkWise: {String(change.error)}
        </p>
      )}
    </div>
  );
}
