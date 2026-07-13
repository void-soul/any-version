import React, { useState } from "react";
import ModelConfig from "./ModelConfig";
import ToolLauncher from "./ToolLauncher";
import UsageStats from "./UsageStats";
import SkillManager from "./SkillManager";
import McpManager from "./McpManager";
import CollabRoom from "./CollabRoom";
import { Settings2, Rocket, BarChart3, Puzzle, Plug, MessagesSquare } from "lucide-react";

type AiSubTab = "model" | "launcher" | "usage" | "skills" | "mcp" | "collab";

const TABS = [
  { key: "model" as AiSubTab, label: "模型", icon: Settings2 },
  { key: "launcher" as AiSubTab, label: "工具", icon: Rocket },
  { key: "skills" as AiSubTab, label: "技能", icon: Puzzle },
  { key: "mcp" as AiSubTab, label: "MCP", icon: Plug },
  { key: "collab" as AiSubTab, label: "协作", icon: MessagesSquare },
  { key: "usage" as AiSubTab, label: "用量", icon: BarChart3 }
];

export default function AiPanel() {
  const [activeTab, setActiveTab] = useState<AiSubTab>("model");
  // 懒挂载：仅渲染至少被访问过一次的 tab，避免全部子组件同时初始化
  const [mountedTabs, setMountedTabs] = useState<Set<AiSubTab>>(new Set(["model"]));
  const switchTab = (tab: AiSubTab) => {
    setActiveTab(tab);
    setMountedTabs((prev) => {
      if (prev.has(tab)) return prev;
      const next = new Set(prev);
      next.add(tab);
      return next;
    });
  };

  return (
    <div className="h-full flex min-h-0 select-none">
      {/* 左侧竖向菜单 */}
      <div className="w-25 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {TABS.map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => switchTab(key)}
            className={`w-full px-3 py-2 rounded-lg text-[11px] font-semibold flex items-center gap-2 transition-all cursor-pointer text-left ${
              activeTab === key
                ? "bg-violet-600 text-white shadow-md shadow-violet-500/10"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Icon className="w-3.5 h-3.5 flex-shrink-0" />
            {label}
          </button>
        ))}
      </div>

      {/* 右侧内容区域 */}
      <div className="flex-1 min-h-0 overflow-hidden">
        {mountedTabs.has("model") && (
          <div className={activeTab === "model" ? "h-full" : "hidden"}>
            <ModelConfig />
          </div>
        )}
        {mountedTabs.has("launcher") && (
          <div className={activeTab === "launcher" ? "h-full" : "hidden"}>
            <ToolLauncher />
          </div>
        )}
        {mountedTabs.has("usage") && (
          <div className={activeTab === "usage" ? "h-full" : "hidden"}>
            <UsageStats />
          </div>
        )}
        {mountedTabs.has("skills") && (
          <div className={activeTab === "skills" ? "h-full" : "hidden"}>
            <SkillManager />
          </div>
        )}
        {mountedTabs.has("mcp") && (
          <div className={activeTab === "mcp" ? "h-full" : "hidden"}>
            <McpManager />
          </div>
        )}
        {mountedTabs.has("collab") && (
          <div className={activeTab === "collab" ? "h-full" : "hidden"}>
            <CollabRoom />
          </div>
        )}
      </div>
    </div>
  );
}
