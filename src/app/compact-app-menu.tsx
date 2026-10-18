import { Menu } from "@base-ui/react/menu";
import { Bot, GitFork, Menu as MenuIcon, Redo2, Settings, Undo2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { refreshPanelAfterClose } from "./eink-refresh";
import { useOverlayInkSuppression } from "./ink-suppression";

export function CompactAppMenu({
  graphOpen,
  onGraph,
  onAssistant,
  onSettings,
  onUndo,
  onRedo,
}: {
  graphOpen: boolean;
  onGraph: () => void;
  onAssistant: () => void;
  onSettings: () => void;
  onUndo?: () => void;
  onRedo?: () => void;
}) {
  const overlay = useOverlayInkSuppression();
  const items = [
    {
      label: graphOpen ? "Close knowledge graph" : "Knowledge graph",
      icon: GitFork,
      action: onGraph,
    },
    { label: "Assistant", icon: Bot, action: onAssistant },
    { label: "Undo structural change", icon: Undo2, action: onUndo },
    { label: "Redo structural change", icon: Redo2, action: onRedo },
    { label: "Settings", icon: Settings, action: onSettings },
  ];
  return (
    <Menu.Root onOpenChange={refreshPanelAfterClose(overlay)}>
      <Menu.Trigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="size-11 rounded-xl"
            aria-label="App menu"
            title="App menu"
          />
        }
      >
        <MenuIcon className="size-5" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner sideOffset={6} align="end" collisionPadding={12} className="z-50">
          <Menu.Popup className="min-w-56 rounded-xl border bg-popover p-1.5 text-popover-foreground shadow-popover outline-none">
            {items.map(({ label, icon: Icon, action }) => (
              <Menu.Item
                key={label}
                disabled={!action}
                onClick={action}
                className="flex min-h-11 cursor-pointer items-center gap-3 rounded-lg px-3 text-sm outline-none data-[highlighted]:bg-accent data-[disabled]:opacity-40"
              >
                <Icon className="size-4" />
                {label}
              </Menu.Item>
            ))}
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}
