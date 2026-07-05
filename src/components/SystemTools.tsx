import React, { useState, useEffect } from "react";
import PortScanner from "./PortScanner";
import EnvBackupManager from "./EnvBackupManager";
import HttpServer from "./HttpServer";
import ImageBase64 from "./ImageBase64";
import PathEnvManager from "./PathEnvManager";
import {
  Network,
  Database,
  Server,
  Image,
  ListOrdered,
} from "lucide-react";

export type SystemToolsTabKey = "ports" | "backups" | "httpServer" | "imageBase64" | "pathEnv";

interface SystemToolsProps {
  defaultTab?: SystemToolsTabKey;
}

const TABS = [
  { key: "ports" as const, label: "端口排查", icon: Network },
  { key: "backups" as const, label: "环境备份", icon: Database },
  { key: "httpServer" as const, label: "HTTP 服务", icon: Server },
  { key: "imageBase64" as const, label: "图片 Base64", icon: Image },
  { key: "pathEnv" as const, label: "PATH 变量", icon: ListOrdered },
];

export default function SystemTools({ defaultTab = "ports" }: SystemToolsProps) {
  const [activeTab, setActiveTab] = useState<SystemToolsTabKey>(defaultTab);

  useEffect(() => {
    if (defaultTab) {
      setActiveTab(defaultTab);
    }
  }, [defaultTab]);

  return (
    <div className="flex-1 overflow-hidden flex min-h-0 select-none">
      {/* 左侧竖向菜单 */}
      <div className="w-40 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {TABS.map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => setActiveTab(key)}
            className={`w-full px-3 py-2 rounded-lg text-[11px] font-semibold flex items-center gap-2 transition-all cursor-pointer text-left ${
              activeTab === key
                ? "bg-emerald-600 text-white shadow-md shadow-emerald-500/10"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Icon className="w-3.5 h-3.5 flex-shrink-0" />
            {label}
          </button>
        ))}
      </div>

      {/* 右侧内容区域 */}
      <div className="flex-1 min-h-0 overflow-y-auto">
        <div className={activeTab === "ports" ? "" : "hidden"}>
          <div className="px-6 py-4"><PortScanner /></div>
        </div>
        <div className={activeTab === "backups" ? "" : "hidden"}>
          <EnvBackupManager />
        </div>
        <div className={activeTab === "httpServer" ? "" : "hidden"}>
          <div className="px-6 py-4 max-w-4xl mx-auto"><HttpServer /></div>
        </div>
        <div className={activeTab === "imageBase64" ? "" : "hidden"}>
          <div className="px-6 py-4 max-w-5xl mx-auto"><ImageBase64 /></div>
        </div>
        <div className={activeTab === "pathEnv" ? "" : "hidden"}>
          <PathEnvManager />
        </div>
      </div>
    </div>
  );
}
