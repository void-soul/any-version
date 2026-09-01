// 环境变量模块：PATH 变量管理（排序/冲突检测）+ 环境变量备份还原（注册表）
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ListOrdered, ShieldCheck } from "lucide-react";
import PathEnvManager from "./PathEnvManager";
import EnvBackupManager from "./EnvBackupManager";

type TabKey = "path" | "backup";

export default function EnvManager() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<TabKey>("path");

  const tabBtn = (key: TabKey, label: string, Icon: any) => (
    <button
      onClick={() => setTab(key)}
      className={`px-3 py-1.5 rounded-md text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
        tab === key ? "text-white shadow-sm" : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
      }`}
      style={tab === key ? { backgroundColor: "var(--module-accent)" } : undefined}
    >
      <Icon className="w-3 h-3" />
      {label}
    </button>
  );

  return (
    <div className="h-full w-full flex flex-col overflow-hidden select-none text-slate-200">
      {/* 模块头部 + 页签 */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5 flex-shrink-0">
        <ShieldCheck className="w-4 h-4 text-[var(--module-accent)]" />
        <span className="text-sm font-bold text-white">{t("envmgr.title")}</span>
        <span className="text-[10px] text-slate-500">{t("envmgr.subtitle")}</span>
        <div className="flex-1" />
        <div className="flex items-center gap-0.5 bg-white/5 border border-white/5 rounded-lg p-0.5">
          {tabBtn("path", t("envmgr.pathTab"), ListOrdered)}
          {tabBtn("backup", t("envmgr.backupTab"), ShieldCheck)}
        </div>
      </div>

      {tab === "path" ? (
        <div className="flex-1 min-h-0 flex flex-col p-5">
          <PathEnvManager />
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-y-auto p-5">
          <div className="max-w-[1100px] mx-auto">
            <EnvBackupManager />
          </div>
        </div>
      )}
    </div>
  );
}
