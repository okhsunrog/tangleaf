import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { AppFindChromeContext } from "@/features/pages/page-find-chrome";
import { onBackButtonPress } from "@tauri-apps/api/app";
import { dismissBackOverlay } from "@/lib/back-overlays";
import { ArrowLeft, X } from "lucide-react";
import { TangleafBrand } from "@/brand/TangleafBrand";
import { Group, Panel, Separator, usePanelRef } from "react-resizable-panels";
import { Button } from "@/components/ui/button";
import { DockVisibility } from "@/features/workspace/workspace-model";
import { cn } from "@/lib/utils";
import {
  CompactRegion,
  CompactBackToHomeContext,
  compactNavigationReducer,
  initialCompactNavigation,
  workbenchIsVisible,
} from "./compact-navigation";
import { useCompactLayout } from "./use-compact-layout";

type Props = {
  /** Before the brand, e.g. window buttons on a desktop that puts them on the left. */
  headerLeading?: ReactNode;
  headerActions?: ReactNode;
  sidebar: ReactNode;
  workbench: ReactNode;
  assistant: ReactNode;
  assistantVisibility: DockVisibility;
  assistantWidth: number;
  onAssistantVisibilityChange: (visibility: DockVisibility) => void;
  onAssistantWidthChange: (width: number) => void;
  workbenchIsHome: boolean;
  editorRequest: number;
  assistantRequest: number;
  searchOpen: boolean;
  onCloseSearch: () => void;
  onNavigateBack: () => void;
  onOpenHome: () => void;
  /**
   * A handwritten note has the screen (sheet mode): the header steps aside, and the note's own
   * toolbar is the only chrome. Android Back keeps working as before.
   */
  sheet?: boolean;
};

export function AppLayout({
  headerLeading,
  headerActions,
  sidebar,
  workbench,
  assistant,
  assistantVisibility,
  assistantWidth,
  onAssistantVisibilityChange,
  onAssistantWidthChange,
  workbenchIsHome,
  editorRequest,
  assistantRequest,
  searchOpen,
  onCloseSearch,
  onNavigateBack,
  onOpenHome,
  sheet = false,
}: Props) {
  const compact = useCompactLayout();
  const [compactNavigation, dispatchCompactNavigation] = useReducer(
    compactNavigationReducer,
    initialCompactNavigation,
  );
  const compactRegion = compactNavigation.region;
  const [findSlot, setFindSlot] = useState<HTMLDivElement | null>(null);
  const [noteFindOpen, setNoteFindOpen] = useState(false);
  const [noteMenuSlot, setNoteMenuSlot] = useState<HTMLDivElement | null>(null);
  const findSlots = useMemo(
    () => ({
      search: findSlot,
      menu: noteMenuSlot,
      visible: workbenchIsVisible(compactRegion) && !searchOpen,
      onSearchOpenChange: setNoteFindOpen,
    }),
    [findSlot, noteMenuSlot, compactRegion, searchOpen],
  );
  const sidebarRef = usePanelRef();
  const workbenchRef = usePanelRef();
  const assistantRef = usePanelRef();
  const assistantVisible = assistantVisibility !== DockVisibility.Hidden;

  useEffect(() => {
    if (editorRequest > 0) dispatchCompactNavigation({ type: "open_editor" });
  }, [editorRequest]);
  useEffect(() => {
    if (assistantRequest > 0) dispatchCompactNavigation({ type: "open_assistant" });
  }, [assistantRequest]);
  useEffect(() => {
    if (workbenchIsHome && compactRegion === CompactRegion.Editor) {
      dispatchCompactNavigation({ type: "open_home" });
    }
  }, [compactRegion, workbenchIsHome]);
  useEffect(() => {
    // A note with the whole screen is the editor, however it got there: a pane tab can show a
    // note while the region is still Home. Without this the note's Back (no pane history) and
    // Android Back would have nowhere to go, and the header and pane tabs are gone.
    if (sheet && compactRegion === CompactRegion.Home) {
      dispatchCompactNavigation({ type: "open_editor" });
    }
  }, [compactRegion, sheet]);
  useEffect(() => {
    if (!assistantVisible && compactRegion === CompactRegion.Assistant) {
      dispatchCompactNavigation({ type: "close_assistant" });
    }
  }, [assistantVisible, compactRegion]);

  const openHome = useCallback(() => dispatchCompactNavigation({ type: "open_home" }), []);
  const returnToJournal = useCallback(() => {
    onOpenHome();
    openHome();
  }, [onOpenHome, openHome]);
  const navigateCompactBack = useCallback(() => {
    if (searchOpen) {
      onCloseSearch();
    } else if (compactRegion === CompactRegion.Assistant) {
      dispatchCompactNavigation({ type: "close_assistant" });
    } else if (compactRegion === CompactRegion.Editor) {
      onNavigateBack();
    }
  }, [compactRegion, onCloseSearch, onNavigateBack, searchOpen]);

  useEffect(() => {
    if (
      !compact ||
      (compactRegion === CompactRegion.Home && !searchOpen && !noteFindOpen) ||
      !navigator.userAgent.toLocaleLowerCase().includes("android")
    ) {
      return;
    }
    let disposed = false;
    let unlisten: (() => Promise<void>) | undefined;
    void onBackButtonPress(() => {
      if (!dismissBackOverlay()) navigateCompactBack();
    })
      .then((listener) => {
        if (disposed) void listener.unregister();
        else unlisten = () => listener.unregister();
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      void unlisten?.();
    };
  }, [compact, compactRegion, navigateCompactBack, searchOpen, noteFindOpen]);

  useEffect(() => {
    // Panel constraints settle after a visibility/breakpoint render. Resizing before
    // that commit can clamp an opening dock against its previous zero maximum.
    const frame = requestAnimationFrame(() => {
      const notes = sidebarRef.current;
      const center = workbenchRef.current;
      const dock = assistantRef.current;
      if (!notes || !center || !dock) return;

      if (compact) {
        if (compactRegion === CompactRegion.Assistant) {
          dock.expand();
          dock.resize("100%");
          notes.collapse();
          center.collapse();
        } else {
          center.expand();
          center.resize("100%");
          notes.collapse();
          dock.collapse();
        }
        return;
      }

      notes.expand();
      center.expand();
      notes.resize("220px");
      if (assistantVisible) {
        dock.expand();
        dock.resize(`${assistantWidth}px`);
      } else {
        dock.collapse();
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [
    assistantRef,
    assistantVisibility,
    assistantVisible,
    assistantWidth,
    compact,
    compactRegion,
    sidebarRef,
    workbenchRef,
  ]);

  const previousAssistantVisible = useRef(assistantVisible);
  useEffect(() => {
    const wasVisible = previousAssistantVisible.current;
    previousAssistantVisible.current = assistantVisible;
    if (assistantVisible || !wasVisible) return;
    let focusFrame = 0;
    const layoutFrame = requestAnimationFrame(() => {
      focusFrame = requestAnimationFrame(() => {
        document.querySelector<HTMLButtonElement>("[data-assistant-toggle]")?.focus();
      });
    });
    return () => {
      cancelAnimationFrame(layoutFrame);
      cancelAnimationFrame(focusFrame);
    };
  }, [assistantVisible]);

  const hideAssistantAndRestoreFocus = () => {
    onAssistantVisibilityChange(DockVisibility.Hidden);
  };

  return (
    <AppFindChromeContext.Provider value={findSlots}>
      <CompactBackToHomeContext.Provider
        value={compact && compactRegion === CompactRegion.Editor ? returnToJournal : null}
      >
        <div className="app-shell flex h-full flex-col overflow-hidden text-foreground">
          {!sheet && (
            <header data-tauri-drag-region className="app-titlebar relative z-20 shrink-0">
              <div ref={setFindSlot} className="app-find-slot" />
              <div
                data-tauri-drag-region
                className="app-titlebar-content flex min-h-14 items-center justify-between gap-1 px-3"
              >
                <div className="flex items-center gap-2">
                  {headerLeading}
                  <button
                    type="button"
                    aria-label="Open journal"
                    className="group -ml-1 flex items-center gap-2.5 rounded-xl px-2 py-1 text-left outline-none transition-colors hover:bg-accent/60 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                    onClick={() => {
                      returnToJournal();
                    }}
                  >
                    <TangleafBrand />
                  </button>
                </div>
                <div className="app-header-actions flex shrink-0 items-center gap-1">
                  {compact && (
                    <div
                      ref={setNoteMenuSlot}
                      hidden={!workbenchIsVisible(compactRegion) || searchOpen}
                    />
                  )}
                  {headerActions}
                </div>
              </div>
            </header>
          )}

          <div
            className={cn(
              "workspace-frame relative mx-3 mb-3 min-h-0 flex-1 overflow-hidden",
              compact && "workspace-compact mx-0 mb-0",
              sheet && "workspace-sheet",
            )}
          >
            <Group orientation="horizontal" className="h-full min-h-0" disabled={compact}>
              <Panel
                id="navigation-sidebar"
                panelRef={sidebarRef}
                // Compact starts collapsed: the effect below collapses it only after the first
                // paint, which used to show the three-column desktop layout for a frame on start.
                defaultSize={compact ? 0 : "220px"}
                minSize={compact ? 0 : "180px"}
                maxSize={compact ? "100%" : "25%"}
                collapsible
                collapsedSize={0}
              >
                <aside
                  aria-hidden={compact ? true : undefined}
                  inert={compact ? true : undefined}
                  className="sidebar-surface h-full overflow-y-auto py-2 pr-3 pl-1"
                >
                  {sidebar}
                </aside>
              </Panel>

              <Separator
                className={cn(
                  "workspace-gutter group relative w-3 bg-transparent after:absolute after:inset-y-4 after:left-1/2 after:w-px hover:after:bg-primary/30",
                  compact && "hidden",
                )}
              />

              <Panel
                id="workbench"
                panelRef={workbenchRef}
                defaultSize={compact ? "100%" : "56%"}
                minSize={compact ? 0 : "38%"}
                collapsible
                collapsedSize={0}
              >
                <main
                  aria-hidden={compact && !workbenchIsVisible(compactRegion) ? true : undefined}
                  inert={compact && !workbenchIsVisible(compactRegion) ? true : undefined}
                  className="canvas-surface h-full overflow-hidden"
                >
                  {workbench}
                </main>
              </Panel>

              <Separator
                className={cn(
                  "workspace-gutter group relative w-3 bg-transparent after:absolute after:inset-y-4 after:left-1/2 after:w-px hover:after:bg-primary/30",
                  (!assistantVisible || compact) && "hidden",
                )}
              />
              <Panel
                id="assistant-dock"
                panelRef={assistantRef}
                defaultSize={compact || !assistantVisible ? 0 : `${assistantWidth}px`}
                minSize={compact || !assistantVisible ? 0 : 320}
                maxSize={compact ? "100%" : assistantVisible ? 480 : 0}
                groupResizeBehavior="preserve-pixel-size"
                disabled={!compact && !assistantVisible}
                collapsible
                collapsedSize={0}
                onResize={(size, _id, previous) => {
                  if (
                    previous &&
                    !compact &&
                    assistantVisibility === DockVisibility.Open &&
                    size.inPixels >= 320
                  ) {
                    onAssistantWidthChange(size.inPixels);
                  }
                }}
              >
                {(assistantVisible || compact) && (
                  <section
                    id="assistant-dock-content"
                    aria-hidden={
                      compact && compactRegion !== CompactRegion.Assistant ? true : undefined
                    }
                    inert={compact && compactRegion !== CompactRegion.Assistant ? true : undefined}
                    className={cn(
                      "inspector-surface flex h-full min-h-0 flex-col overflow-hidden p-3",
                      compact && "pb-[calc(0.75rem+var(--safe-area-inset-bottom))]",
                    )}
                  >
                    <div
                      className={cn("mb-1 flex h-7 justify-end gap-1", compact && "justify-start")}
                    >
                      {compact && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="xs"
                          aria-label="Back from Assistant"
                          onClick={() => dispatchCompactNavigation({ type: "close_assistant" })}
                          className="gap-1 rounded-lg px-1.5"
                        >
                          <ArrowLeft className="size-3.5" />
                          Back
                        </Button>
                      )}
                      {!compact && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          aria-label="Hide Assistant"
                          onClick={hideAssistantAndRestoreFocus}
                        >
                          <X className="size-3.5" />
                        </Button>
                      )}
                    </div>
                    <div className="min-h-0 flex-1">{assistant}</div>
                  </section>
                )}
              </Panel>
            </Group>
          </div>
        </div>
      </CompactBackToHomeContext.Provider>
    </AppFindChromeContext.Provider>
  );
}
