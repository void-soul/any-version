import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { alertError } from "../shared/ThemedAlert";

/**
 * 「未检测到安装目录」空状态下的动作组。
 *
 * 主按钮是「手动指定目录」（接管用户自己下的那份）。当项目其实配好了在线版本来源时
 * 再加一个「开启在线下载」：`delegation.version_control` 是持久化的，历史上以简单托管
 * 方式托管过的项目会一直是 false（Redis / MySQL 都是这类），不翻转它就只能永远手动放目录。
 */
export function ProjectInstallActions({ id, hasVersionSupport, onPickDir, onDone }: {
  id: string;
  hasVersionSupport: boolean;
  onPickDir: () => void;
  onDone: () => void | Promise<void>;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  const enable = useCallback(async () => {
    setBusy(true);
    try {
      await invoke("project_enable_version_control", { id });
      await onDone();
    } catch (e) {
      alertError(String(e));
    } finally {
      setBusy(false);
    }
  }, [id, onDone]);

  return (
    <div className="flex items-center gap-2">
      <button
        onClick={onPickDir}
        className="px-5 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white rounded-card text-body font-semibold shadow-lg shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5 hover:scale-[1.02] active:scale-[0.98]"
      >
        {t("projdetail.manualSpecifyDir")}
      </button>
      {hasVersionSupport && (
        <button
          onClick={() => void enable()}
          disabled={busy}
          title={t("projdetail.enableVersionControlDesc")}
          className="px-4 py-2.5 bg-white/10 hover:bg-white/15 disabled:opacity-50 text-slate-200 rounded-card text-body font-semibold border border-white/10 cursor-pointer transition-all"
        >
          {t("projdetail.enableVersionControl")}
        </button>
      )}
    </div>
  );
}
