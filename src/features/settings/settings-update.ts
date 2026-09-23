import type { SecretKey, SettingsSnapshot, SettingsUpdate } from "@/lib/api";

export function toSettingsUpdate(
  settings: SettingsSnapshot,
  apiKeys: Partial<Record<SecretKey, string>> = {},
  clearKeys: SecretKey[] = [],
): SettingsUpdate {
  return {
    windowCornerRadius: settings.windowCornerRadius,
    windowDecorationMode: settings.windowDecorationMode,
    windowControlsStyle: settings.windowControlsStyle,
    startupView: settings.startupView,
    startupPageUuid: settings.startupPageUuid,
    syncServerUrl: settings.syncServerUrl,
    aiSearchEnabled: settings.aiSearchEnabled,
    aiSearchTrigger: settings.aiSearchTrigger,
    aiSearchRerank: settings.aiSearchRerank,
    searchDebugSources: settings.searchDebugSources,
    apiKeys,
    clearKeys,
  };
}
