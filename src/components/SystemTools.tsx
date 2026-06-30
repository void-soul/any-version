import React, { useState, useEffect } from "react";
import PortScanner from "./PortScanner";
import EnvBackupManager from "./EnvBackupManager";
import HttpServer from "./HttpServer";
import ImageBase64 from "./ImageBase64";
import PathEnvManager from "./PathEnvManager";
import RssReader from "./RssReader";
import { 
  Network, 
  Database, 
  Server, 
  Image,
  ListOrdered,
  Rss
} from "lucide-react";

export type SystemToolsTabKey = "ports" | "backups" | "httpServer" | "imageBase64" | "pathEnv" | "rss";

interface SystemToolsProps {
  defaultTab?: SystemToolsTabKey;
}

export default function SystemTools({ defaultTab = "ports" }: SystemToolsProps) {
  const [activeTab, setActiveTab] = useState<SystemToolsTabKey>(defaultTab);

  useEffect(() => {
    if (defaultTab) {
      setActiveTab(defaultTab);
    }
  }, [defaultTab]);

  const tabs = [
    { key: "ports" as const, label: "端口排查", icon: Network },
    { key: "backups" as const, label: "环境备份", icon: Database },
    { key: "httpServer" as const, label: "HTTP 服务", icon: Server },
    { key: "imageBase64" as const, label: "图片 Base64", icon: Image },
    { key: "pathEnv" as const, label: "PATH 变量排列", icon: ListOrdered },
    { key: "rss" as const, label: "资讯", icon: Rss },
  ];

  return (
    <div className="flex-1 overflow-hidden flex flex-col select-none">
      {/* 顶部 Tab 栏 */}
      <div className="px-6 pt-2 pb-3 flex items-center gap-2 flex-shrink-0 border-b border-white/5 overflow-x-auto whitespace-nowrap scrollbar-none">
        {tabs.map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => setActiveTab(key)}
            className={`px-3 py-1.5 rounded-lg text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
              activeTab === key 
                ? "bg-blue-600 text-white shadow-md shadow-blue-500/10" 
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Icon className="w-3.5 h-3.5" />
            {label}
          </button>
        ))}
      </div>

      {/* 内容区域 */}
      <div className="flex-1 min-h-0 relative">
        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 overflow-y-auto ${activeTab === "ports" ? "" : "hidden"}`}>
          <div className="max-w-3xl"><PortScanner /></div>
        </div>
        
        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 ${activeTab === "backups" ? "" : "hidden"}`}>
          <EnvBackupManager />
        </div>

        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 overflow-y-auto ${activeTab === "httpServer" ? "" : "hidden"}`}>
          <div className="max-w-4xl mx-auto w-full"><HttpServer /></div>
        </div>

        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 overflow-y-auto ${activeTab === "imageBase64" ? "" : "hidden"}`}>
          <div className="max-w-5xl mx-auto w-full"><ImageBase64 /></div>
        </div>

        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 ${activeTab === "pathEnv" ? "" : "hidden"}`}>
          <PathEnvManager />
        </div>

        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 ${activeTab === "rss" ? "" : "hidden"}`}>
          <RssReader />
        </div>
      </div>
    </div>
  );
}
