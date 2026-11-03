import { FILTER_PRESETS } from "../FileSearchPopup.model";
export interface FileSearchScope { drive: string; filter: string; }
export function fileSearchRequest(query: string, scope: FileSearchScope) {
  const preset = FILTER_PRESETS.find((item) => item.id === scope.filter) ?? FILTER_PRESETS[0];
  const drives = scope.drive ? [scope.drive] : [];
  const options = { query: query.trim(), limit: 30, drives, kind: preset.kind, exts: preset.exts };
  return { options, key: JSON.stringify(options) };
}
