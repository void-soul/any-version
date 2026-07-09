import React, { useState } from "react";
import ModelConfig from "./ModelConfig";
import ToolLauncher from "./ToolLauncher";
import UsageStats from "./UsageStats";
import SkillManager from "./SkillManager";
import McpManager from "./McpManager";
import { Settings2, Rocket, BarChart3, Puzzle, Plug } from "lucide-react";

type AiSubTab = "model" | "launcher" | "usage" | "skills" | "mcp";

const TABS = [
  { key: "model" as AiSubTab, label: "模型配置", icon: Settings2 },
  { key: "launcher" as AiSubTab, label: "工具启动", icon: Rocket },
  { key: "usage" as AiSubTab, label: "用量统计", icon: BarChart3 },
  { key: "skills" as AiSubTab, label: "技能管理", icon: Puzzle },
  { key: "mcp" as AiSubTab, label: "MCP 管理", icon: Plug },
];

export default function AiPanel() {
  const [activeTab, setActiveTab] = useState<AiSubTab>("model");

  return (
    <div className="h-full flex min-h-0 select-none">
      {/* 左侧竖向菜单 */}
      <div className="w-40 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {TABS.map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => setActiveTab(key)}
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
        <div className={activeTab === "model" ? "h-full" : "hidden"}>
          <ModelConfig />
        </div>
        <div className={activeTab === "launcher" ? "h-full" : "hidden"}>
          <ToolLauncher />
        </div>
        <div className={activeTab === "usage" ? "h-full" : "hidden"}>
          <UsageStats />
        </div>
        <div className={activeTab === "skills" ? "h-full" : "hidden"}>
          <SkillManager />
        </div>
        <div className={activeTab === "mcp" ? "h-full" : "hidden"}>
          <McpManager />
        </div>
      </div>
    </div>
  );
}
