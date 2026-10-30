import { TOOL_ICON_FALLBACK, TOOL_ICON_RULES } from "./CollabRoom.model";
export function ToolActivityIcon({ activity }: { activity: string }) {
  const text = activity ?? "";
  for (const [re, Icon] of TOOL_ICON_RULES) {
    if (re.test(text)) return <Icon className="w-2.5 h-2.5 flex-shrink-0" />;
  }
  return <TOOL_ICON_FALLBACK className="w-2.5 h-2.5 flex-shrink-0" />;
}
