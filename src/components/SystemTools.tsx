import React, { useState } from "react";
import HostsManager from "./HostsManager";
import PortScanner from "./PortScanner";
import { Terminal, Network, ShieldCheck } from "lucide-react";

export default function SystemTools() {
  const [activeTab, setActiveTab] = useState<"hosts" | "ports">("hosts");

  return (
    <div className="flex-1 overflow-hidden h-screen flex flex-col">
      {/* Tab Switcher at Top */}
      <div className="p-8 pb-2 flex items-center justify-between flex-shrink-0 select-none">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide font-sans">系统实用工具</h2>
          <p className="text-xs text-slate-400 mt-1">本地 hosts 网络映射及排查独占端口占用分析</p>
        </div>

        <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5">
          <button
            onClick={() => setActiveTab("hosts")}
            className={`px-4 py-2 rounded-lg text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer ${
              activeTab === "hosts" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            <ShieldCheck className="w-4 h-4" />
            Hosts 解析编辑
          </button>
          
          <button
            onClick={() => setActiveTab("ports")}
            className={`px-4 py-2 rounded-lg text-xs font-semibold flex items-center gap-1.5 transition-all cursor-pointer ${
              activeTab === "ports" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            <Network className="w-4 h-4" />
            TCP 端口排查
          </button>
        </div>
      </div>

      {/* Content wrapper */}
      <div className="flex-1 min-h-0 overflow-y-auto px-8 pb-8">
        {activeTab === "hosts" ? (
          <div className="-mx-8">
            <HostsManager />
          </div>
        ) : (
          <div className="max-w-3xl mt-4">
            <PortScanner />
          </div>
        )}
      </div>
    </div>
  );
}
