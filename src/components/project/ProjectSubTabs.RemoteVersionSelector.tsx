import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, RefreshCw, Download, Search } from "lucide-react";
// ═══════════════════════════════════════
//  远程版本选择器（可搜索过滤）
// ═══════════════════════════════════════
export function RemoteVersionSelector({
  remoteVersions,
  loadingRemote,
  installingVersion,
  isOperating,
  onInstall,
  versionsUpdatedAt,
  onRefresh,
  disabled = false,
  disabledReason = "",
}: {
  remoteVersions: string[];
  loadingRemote: boolean;
  installingVersion: string | null;
  isOperating?: boolean;
  onInstall: (version: string) => void;
  versionsUpdatedAt?: number | null;
  onRefresh?: () => void;
  disabled?: boolean;
  disabledReason?: string;
}) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [open, setOpen] = useState(false);
  const containerRef = React.useRef<HTMLDivElement>(null);

  const filtered = search.trim()
    ? remoteVersions.filter((v) => v.toLowerCase().includes(search.toLowerCase()))
    : remoteVersions;

  // 格式化上次更新时间
  const formatUpdatedAt = (ts: number | null | undefined): string => {
    if (!ts) return "";
    const diff = Math.floor(Date.now() / 1000) - ts;
    if (diff < 60) return t("projsub.justNow");
    if (diff < 3600) return t("projsub.minAgo", { n: Math.floor(diff / 60) });
    if (diff < 86400) return t("projsub.hrAgo", { n: Math.floor(diff / 3600) });
    const d = new Date(ts * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };

  // 点击外部关闭下拉
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const handleSelect = (v: string) => {
    setSearch(v);
    setOpen(false);
  };

  const handleInstall = () => {
    if (search.trim() && remoteVersions.includes(search.trim())) {
      onInstall(search.trim());
      setSearch("");
    }
  };

  return (
    <div className="space-y-3 border-t border-white/5 pt-4">
      {/* 标题行：含上次更新时间和刷新按钮 */}
      <div className="flex items-center justify-between">
        <div>
          <h4 className="text-body font-semibold text-slate-300">{t("projsub.remoteInstallTitle")}</h4>
          <p className="text-title text-slate-500 mt-0.5">{t("projsub.remoteInstallDesc")}</p>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          {versionsUpdatedAt && !loadingRemote && (
            <span className="text-caption text-slate-600">
              {t("projsub.lastUpdate", { time: formatUpdatedAt(versionsUpdatedAt) })}
            </span>
          )}
          {onRefresh && (
            <button
              onClick={onRefresh}
              disabled={disabled || loadingRemote || !!installingVersion}
              title={disabled ? disabledReason : t("projsub.refreshList")}
              className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed text-slate-300 rounded-ctl text-caption border border-white/8 cursor-pointer transition-all"
            >
              <RefreshCw className={`w-3 h-3 ${loadingRemote ? "animate-spin text-[var(--module-accent)]" : ""}`} />
              {loadingRemote ? t("projsub.updating") : t("projsub.updateList")}
            </button>
          )}
        </div>
      </div>

      {loadingRemote && remoteVersions.length === 0 ? (
        <div className="flex items-center gap-2 text-slate-400 text-body py-2">
          <RefreshCw className="w-4 h-4 animate-spin text-[var(--module-accent)]" />
          {t("projsub.fetchingRemote")}
        </div>
      ) : (
        <div className="space-y-2">
          {disabled && disabledReason && (
            <div className="p-3 rounded-card border border-amber-500/15 bg-amber-500/5 text-amber-400 text-caption mb-2 leading-relaxed flex items-center gap-1.5 animate-fadeIn">
              <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0" />
              <span>{disabledReason}</span>
            </div>
          )}
          <div ref={containerRef} className="relative">
            <div className="flex items-center gap-3">
              <div className="relative flex-1">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
                <input
                  type="text"
                  value={search}
                  disabled={disabled}
                  onChange={(e) => { setSearch(e.target.value); setOpen(true); }}
                  onFocus={() => setOpen(true)}
                  placeholder={disabled ? disabledReason : t("projsub.remoteFilterPh")}
                  className="w-full glass-input pl-9 pr-9 py-2 text-body"
                />
                {search && (
                  <button
                    onClick={() => { setSearch(""); setOpen(true); }}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300 cursor-pointer"
                  >
                    <span className="text-body">×</span>
                  </button>
                )}
              </div>
              <button
                onClick={handleInstall}
                disabled={disabled || installingVersion !== null || isOperating || !search.trim() || !remoteVersions.includes(search.trim())}
                title={disabled ? disabledReason : t("projsub.oneClickInstall")}
                className="px-5 py-2 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-card text-body font-semibold shadow-md shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Download className="w-3.5 h-3.5" />
                {installingVersion ? t("projsub.installingVer") : t("projsub.oneClickInstall")}
              </button>
            </div>

            {/* 下拉列表 */}
            {open && filtered.length > 0 && (
              <div className="absolute z-50 mt-1 w-full max-h-48 overflow-y-auto glass-panel rounded-card border border-white/10 bg-surface-panel shadow-2xl">
                {filtered.map((v) => (
                  <button
                    key={v}
                    onClick={() => handleSelect(v)}
                    className={`w-full text-left px-3 py-1.5 text-body hover:bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] transition-colors cursor-pointer ${search.trim() === v ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "text-slate-300"
                      }`}
                  >
                    {v}
                  </button>
                ))}
              </div>
            )}

            {/* 无匹配提示 */}
            {open && search.trim() && filtered.length === 0 && (
              <div className="absolute z-50 mt-1 w-full glass-panel rounded-card border border-white/10 bg-surface-panel shadow-2xl p-3 text-center">
                <p className="text-title text-slate-500">{t("projsub.noMatch", { kw: search })}</p>
              </div>
            )}
          </div>

          {/* 版本统计 */}
          <p className="text-body text-slate-600">
            {t("projsub.remoteCount", {
              count: remoteVersions.length,
              extra: search.trim() && filtered.length !== remoteVersions.length ? t("projsub.matchCount", { count: filtered.length }) : "",
            })}
          </p>
        </div>
      )}
    </div>
  );
}
