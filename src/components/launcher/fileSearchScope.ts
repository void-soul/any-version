import { FILTER_PRESETS } from "../FileSearchPopup.model";
export interface FileSearchScope { drive: string; filter: string; source?: string; }
export function fileSearchRequest(query: string, scope: FileSearchScope) {
  const preset = FILTER_PRESETS.find((item) => item.id === scope.filter) ?? FILTER_PRESETS[0];
  const drives = scope.drive ? [scope.drive] : [];
  const options = { query: query.trim(), limit: 30, drives, kind: preset.kind, exts: preset.exts };
  return { options, key: JSON.stringify(options) };
}

export const searchesFiles = (scope: FileSearchScope) => !scope.source || scope.source === "all" || scope.source === "files";
export const searchesLauncher = (scope: FileSearchScope) => !scope.source || scope.source === "all" || scope.source === "launcher";

export function webSearchUrl(template: string, term: string) {
  const encoded = encodeURIComponent(term);
  return template.includes("%s") ? template.replace("%s", encoded) : `${template}${encoded}`;
}
