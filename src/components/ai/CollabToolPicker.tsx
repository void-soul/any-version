import { useTranslation } from "react-i18next";
import type { DetectedAiTool, CollabAgentStatus } from "../../contracts/ai";
import { agentDotClass, agentStatusTitle, getToolAvatar, getToolNickname } from "./CollabRoom.model";
import type { Dispatch, SetStateAction } from "react";

interface CollabToolPickerProps {
  toolsLoading: boolean;
  tools: DetectedAiTool[];
  selectedTool: string;
  agents: Record<string, CollabAgentStatus>;
  setSelectedTool: Dispatch<SetStateAction<string>>;
}

/** Pure CollabToolPicker view; its owning panel retains side effects and state. */
export function CollabToolPicker({ toolsLoading, tools, selectedTool, agents, setSelectedTool }: CollabToolPickerProps) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap gap-1 items-center">
      <span className="text-micro text-slate-500 mr-1">{t("collab.atTool")}</span>
      {toolsLoading && <span className="text-micro text-slate-400 animate-pulse">{t("collab.loading")}</span>}
      {/* 协同只带得上 CLI 工具：桌面端是 GUI 程序，@ 它没法接任务
          （后端 dispatch 也会拦一道，这里先在 UI 上不让它出现） */}
      {tools.filter((tl) => tl.installed && (tl.tool_kind ?? "other") === "cli").map((tl) => {
        const on = selectedTool === tl.id;
        const avatar = getToolAvatar(tools, tl.id);
        const nickname = getToolNickname(tools, tl.id) || tl.display_name;
        return (
          <div key={tl.id} className="relative flex items-center">
            <span
              className={`absolute -top-0.5 -right-0.5 z-10 w-2 h-2 rounded-full border border-slate-900 ${agentDotClass(agents[tl.id]?.status)}`}
              title={agentStatusTitle(t, agents[tl.id]?.status)}
            />
            <button
              onClick={() => setSelectedTool(on ? "" : tl.id)}
              className={`px-2 py-0.5 rounded-full text-micro font-semibold transition-all cursor-pointer ${
                on
                  ? "bg-[var(--module-accent)] text-white"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border border-white/10"
              }`}
            >
              {avatar ? `${avatar} ` : ""}{nickname}
            </button>
          </div>
        );
      })}
    </div>
  );
}
