import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "next-themes";
import App from "./App";
import { AppearanceProvider } from "@/app/appearance";
import { ConfirmationProvider } from "@/app/confirmation";
import { ErrorBoundary } from "@/app/error-boundary";
import { installTextFocusSuppression } from "@/app/ink-suppression";
import { useSheetMode } from "@/app/sheet-mode";
import { registerLifecycleFlush } from "@/app/lifecycle-flush";
import { disableViewportZoom } from "@/app/viewport-zoom";
import { PageSessionProvider, PageSessionRegistry } from "@/features/pages/page-session";
import { InputCapabilitiesProvider } from "@/features/handwriting/input-capabilities";
import { Toaster } from "@/components/ui/sonner";
import "./index.css";

import { createAppQueryClient, listenForDomainEvents } from "@/lib/query";
import { UI_PROFILE_ENABLED, onUiProfileRender } from "@/lib/ui-profile";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

const queryClient = createAppQueryClient();
void listenForDomainEvents(queryClient);
disableViewportZoom();
// The firmware pen has to be down before the soft keyboard is up, and only the page knows a DOM
// field took the caret. Registered for the whole document, not per editor.
installTextFocusSuppression();
// Owned here rather than by the provider so the process-level drain can reach
// unsaved drafts without a React tree.
const pageSessions = new PageSessionRegistry();
registerLifecycleFlush(pageSessions);

const application = (
  <React.StrictMode>
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
      <QueryClientProvider client={queryClient}>
        <AppearanceProvider>
          <ConfirmationProvider>
            <ErrorBoundary>
              <PageSessionProvider registry={pageSessions}>
                <InputCapabilitiesProvider>
                  <App />
                </InputCapabilitiesProvider>
              </PageSessionProvider>
            </ErrorBoundary>
          </ConfirmationProvider>
          {/* Inside the appearance provider: the toaster's duration and icons follow the
              resolved display profile. */}
          <AppToaster />
        </AppearanceProvider>
      </QueryClientProvider>
    </ThemeProvider>
  </React.StrictMode>
);

root.render(
  UI_PROFILE_ENABLED ? (
    <React.Profiler id="app" onRender={onUiProfileRender}>
      {application}
    </React.Profiler>
  ) : (
    application
  ),
);

/**
 * Bottom right, except while a handwritten note has the screen: there the corner is sheet, a pen
 * tap on a toast action would be taken by the ink routing, and the firmware ink would draw over
 * the toast. In sheet mode toasts cover the toolbar row at the top instead.
 *
 * One toaster whose props change, not two: a remounted toaster starts empty, so a toast on screen
 * when a note opens would vanish instead of moving up.
 */
function AppToaster() {
  const sheet = useSheetMode((state) => state.sheet);
  return (
    <Toaster
      position={sheet ? "top-center" : "bottom-right"}
      offset={sheet ? { top: 4 } : undefined}
      mobileOffset={
        sheet
          ? { top: 4 }
          : {
              right: "calc(1rem + var(--safe-area-inset-right))",
              bottom: "calc(1rem + var(--safe-area-inset-bottom))",
              left: "calc(1rem + var(--safe-area-inset-left))",
            }
      }
    />
  );
}
