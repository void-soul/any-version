import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ShieldCheck, Trash2, X } from "lucide-react";
import { reportInvokeError } from "../../utils/invokeOrReport";

interface TrustedHost {
  host: string;
  port: number;
  fingerprint: string;
}

const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-tiny border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";

/** 已信任主机管理：查看指纹、撤销信任（撤销后下次连接会重新要求确认）。 */
export function TrustedHostsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [list, setList] = useState<TrustedHost[]>([]);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    try { setList(await invoke<TrustedHost[]>("ssh_list_trusted_hosts")); }
    catch (e) { setError(reportInvokeError(e, "ssh_list_trusted_hosts")); }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const forget = async (item: TrustedHost) => {
    if (!window.confirm(t("trust.confirmForget", { host: item.host, port: item.port }))) return;
    try {
      setList(await invoke<TrustedHost[]>("ssh_forget_host", { host: item.host, port: item.port }));
    } catch (e) { setError(reportInvokeError(e, "ssh_forget_host")); }
  };

  return (
    <div className="fixed inset-0 z-[140] flex items-center justify-center bg-black/70 p-4">
      <div className="flex max-h-[80vh] w-[560px] max-w-full flex-col rounded border border-white/10 bg-slate-900 p-4 text-slate-200 shadow-2xl">
        <div className="mb-3 flex items-center gap-2">
          <ShieldCheck className="h-4 w-4 text-emerald-400" />
          <h3 className="text-caption font-semibold">{t("trust.title")}</h3>
          <button className={`${btn} ml-auto`} onClick={onClose}><X className="h-3.5 w-3.5" /></button>
        </div>

        {error && <div className="mb-2 rounded border border-rose-400/30 bg-rose-400/10 px-3 py-2 text-tiny text-rose-200">{error}</div>}

        <div className="min-h-0 flex-1 overflow-auto rounded border border-white/10">
          <table className="w-full text-tiny">
            <thead className="sticky top-0 bg-slate-900/90 text-slate-400">
              <tr>
                <th className="px-3 py-2 text-left">{t("rdp.host")}</th>
                <th className="px-3 py-2 text-left">{t("rdp.port")}</th>
                <th className="px-3 py-2 text-left">{t("trust.fingerprint")}</th>
                <th className="px-3 py-2 text-left">{t("rdp.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {list.length === 0 && (
                <tr><td colSpan={4} className="px-3 py-8 text-center text-slate-500">{t("trust.empty")}</td></tr>
              )}
              {list.map((item) => (
                <tr key={`${item.host}:${item.port}`} className="border-t border-white/5">
                  <td className="px-3 py-2">{item.host}</td>
                  <td className="px-3 py-2">{item.port}</td>
                  <td className="px-3 py-2 font-mono text-micro text-slate-400 break-all">{item.fingerprint}</td>
                  <td className="px-3 py-2">
                    <button className={btn} title={t("trust.forget")} onClick={() => void forget(item)}>
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="mt-2 text-micro text-slate-500">{t("trust.hint")}</p>
      </div>
    </div>
  );
}
