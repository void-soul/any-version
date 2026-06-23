import React, { useState } from "react";
import HostsManager from "./HostsManager";
import PortScanner from "./PortScanner";
import { Network, ShieldCheck, Database } from "lucide-react";
import EnvBackupManager from "./EnvBackupManager";

export default function SystemTools() {
  const [activeTab, setActiveTab] = useState<"hosts" | "ports" | "backups">("hosts");

  return (
    <div className="flex-1 overflow-hidden flex flex-col select-none">
      <div className="px-6 pt-2 pb-3 flex items-center gap-2 flex-shrink-0 border-b border-white/5">
        {[
          { key: "hosts" as const, label: "Hosts", icon: ShieldCheck },
          { key: "ports" as const, label: "端口排查", icon: Network },
          { key: "backups" as const, label: "环境备份", icon: Database },
        ].map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => setActiveTab(key)}
            className={`px-3 py-1.5 rounded-lg text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${activeTab === key ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
              }`}
          >
            <Icon className="w-3 h-3" />
            {label}
          </button>
        ))}
      </div>

      <div className="flex-1 min-h-0 relative">
        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 ${activeTab === "hosts" ? "" : "hidden"}`}>
          <HostsManager />
        </div>
        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 ${activeTab === "ports" ? "" : "hidden"}`}>
          <div className="flex-1 min-h-0 overflow-y-auto pr-1">
            <div className="max-w-3xl"><PortScanner /></div>
          </div>
        </div>
        <div className={`absolute inset-0 flex flex-col min-h-0 px-6 py-4 ${activeTab === "backups" ? "" : "hidden"}`}>
          <EnvBackupManager />
        </div>
      </div>
    </div>
  );
}
