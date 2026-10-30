import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Loader2, Save, Eraser, FolderOpen } from "lucide-react";
import { type BuddyClientPath, type Tab } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the BuddyClientPaths feature, independently of page rendering. */
export function useBuddyClientPaths({ setMessage, tab, showMsg }: {
  setMessage: Dispatch<SetStateAction<{ ok: boolean; text: string; } | null>>;
  tab: Tab;
  showMsg: (ok: boolean, text: string) => void;
}) {
  const { t } = useTranslation();

  // 客户端路径设置（切换时关闭/重启的 WorkBuddy / CodeBuddy CN）
  const [clientPaths, setClientPaths] = useState<BuddyClientPath[]>([]);

  const [pathDraft, setPathDraft] = useState<Record<string, string>>({});

  const [pathBusy, setPathBusy] = useState(false);

  // 客户端路径设置加载 / 保存 / 清除
  const loadClientPaths = useCallback(async () => {
    try {
      const list = await invoke<BuddyClientPath[]>("buddy_get_client_paths");
      setClientPaths(list);
      setPathDraft({});
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  }, []);

  useEffect(() => {
    if (tab === "settings") void loadClientPaths();
  }, [tab, loadClientPaths]);

  const saveClientPath = async (platform: string, path: string) => {
    setPathBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_set_client_path", { platform, path });
      showMsg(true, path.trim() ? t("buddy.clientPaths.saved") : t("buddy.clientPaths.cleared"));
      await loadClientPaths();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setPathBusy(false);
    }
  };

  const browseClientPath = async (entry: BuddyClientPath) => {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Executable", extensions: ["exe", "app"] }],
      defaultPath: entry.configured ?? entry.resolved ?? undefined,
    });
    if (typeof selected === "string" && selected) {
      setPathDraft((prev) => ({ ...prev, [entry.platform]: selected }));
    }
  };

const renderClientPaths = () => (<div className="rounded-card border border-white/10 bg-white/[0.03] p-4">
            <div className="text-title font-bold text-white mb-1">{t("buddy.clientPaths.title")}</div>
            <p className="text-micro text-slate-600 mb-3">{t("buddy.clientPaths.hint")}</p>
            <div className="space-y-3">
              {clientPaths.map((entry) => {
                const draft = pathDraft[entry.platform];
                const current = draft ?? entry.configured ?? "";
                const dirty = current !== (entry.configured ?? "");
                return (
                  <div key={entry.platform} className="rounded-ctl border border-white/10 bg-black/20 p-2.5">
                    <div className="flex items-center gap-2 mb-1.5">
                      <span className="text-caption font-semibold text-white">{entry.label}</span>
                      <span
                        className={`px-1.5 py-0.5 rounded text-micro border ${
                          entry.configured
                            ? "text-sky-300 bg-sky-500/10 border-sky-500/25"
                            : entry.resolved
                              ? "text-emerald-300 bg-emerald-500/10 border-emerald-500/25"
                              : "text-slate-500 bg-white/5 border-white/10"
                        }`}
                      >
                        {entry.configured
                          ? t("buddy.clientPaths.custom")
                          : entry.resolved
                            ? t("buddy.clientPaths.auto")
                            : t("buddy.clientPaths.missing")}
                      </span>
                    </div>
                    <input
                      value={current}
                      onChange={(e) =>
                        setPathDraft((prev) => ({ ...prev, [entry.platform]: e.target.value }))
                      }
                      placeholder={entry.resolved ?? t("buddy.clientPaths.placeholder")}
                      spellCheck={false}
                      className="w-full bg-black/30 border border-white/10 rounded-ctl px-2 py-1.5 text-caption text-white outline-none focus:border-[var(--module-accent)]/50"
                    />
                    {entry.resolved && (
                      <div className="text-micro text-slate-600 mt-1 break-all">
                        {t("buddy.clientPaths.resolved")}: {entry.resolved}
                      </div>
                    )}
                    <div className="flex gap-2 mt-2">
                      <button
                        onClick={() => void browseClientPath(entry)}
                        className="px-2.5 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
                      >
                        <FolderOpen className="w-3 h-3" /> {t("buddy.clientPaths.browse")}
                      </button>
                      <button
                        onClick={() => void saveClientPath(entry.platform, current)}
                        disabled={pathBusy || !dirty}
                        className="px-2.5 py-1 rounded-md text-tiny ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                      >
                        {pathBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}{" "}
                        {t("buddy.clientPaths.save")}
                      </button>
                      {entry.configured && (
                        <button
                          onClick={() => void saveClientPath(entry.platform, "")}
                          disabled={pathBusy}
                          className="px-2.5 py-1 rounded-md text-tiny bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                        >
                          <Eraser className="w-3 h-3" /> {t("buddy.clientPaths.clear")}
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>);
  return {
    renderClientPaths,
  };
}
