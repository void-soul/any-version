import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Loader2, Network, RefreshCw } from "lucide-react";
import VexEmptyState from "../VexEmptyState";

interface NetConnection {
  proto: string;
  local: string;
  remote: string;
  state: string;
  pid: string;
  process: string;
}

export default function NetworkConnections() {
  const { t } = useTranslation();
  const [conns, setConns] = useState<NetConnection[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setConns(await invoke<NetConnection[]>("net_connections"));
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  // 切到「连接」页签即加载，不再要求先点一次刷新
  useEffect(() => {
    void load();
  }, [load]);

  const filtered = useMemo(() => {
    if (!conns) return [];
    const f = filter.trim().toLowerCase();
    if (!f) return conns;
    return conns.filter(
      (c) =>
        c.local.toLowerCase().includes(f) ||
        c.remote.toLowerCase().includes(f) ||
        c.state.toLowerCase().includes(f) ||
        c.process.toLowerCase().includes(f) ||
        c.proto.toLowerCase().includes(f),
    );
  }, [conns, filter]);

  return (
    <div className="mx-auto flex h-full min-h-0 max-w-[1400px] flex-col">
      <div className="ui-card flex min-h-0 flex-1 flex-col p-4">
        <div className="flex shrink-0 items-center gap-2 border-b border-white/5 pb-2">
          <Network className="h-4 w-4 text-blue-400" />
          <h4 className="text-body font-semibold text-white">{t("netpan.connsTitle")}</h4>
          {conns && (
            <span className="text-micro text-slate-500">
              {t("netpan.connCount", { filtered: filtered.length, total: conns.length })}
            </span>
          )}
          <div className="flex-1" />
          <button
            onClick={() => void load()}
            disabled={loading}
            className="ui-btn px-3 h-8"
          >
            {loading ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <RefreshCw className="h-3 w-3" />
            )}
            {conns ? t("netpan.refresh") : t("netpan.loadConns")}
          </button>
        </div>

        {error && (
          <div className="ui-note ui-note-error mt-3">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        {conns && (
          <div className="mt-3 flex min-h-0 flex-1 flex-col gap-2">
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              className="ui-input w-full shrink-0 h-8 px-3"
              placeholder={t("netpan.filterPh")}
            />
            <div className="min-h-0 flex-1 overflow-y-auto">
              {filtered.length === 0 ? (
                <VexEmptyState
                  title={t("netpan.noMatch")}
                  desc={t("netpan.noMatchDesc")}
                  avatarSize={36}
                />
              ) : (
                <table className="w-full text-tiny">
                  <thead className="sticky top-0 bg-surface-deep">
                    <tr className="border-b border-white/5 font-semibold text-slate-400">
                      <td className="py-1.5 pr-3">{t("netpan.thProto")}</td>
                      <td className="py-1.5 pr-3">{t("netpan.thLocal")}</td>
                      <td className="py-1.5 pr-3">{t("netpan.thRemote")}</td>
                      <td className="py-1.5 pr-3">{t("netpan.thState")}</td>
                      <td className="py-1.5">{t("netpan.thProc")}</td>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-white/[0.03] text-slate-300">
                    {filtered.map((c, i) => (
                      <tr key={`${c.proto}-${c.local}-${c.pid}-${i}`} className="hover:bg-white/[0.02]">
                        <td className="py-1 font-mono text-slate-400">{c.proto}</td>
                        <td className="py-1 font-mono">{c.local}</td>
                        <td className="py-1 font-mono">{c.remote}</td>
                        <td className="py-1">
                          <span
                            className={
                              c.state === "LISTENING"
                                ? "text-amber-400"
                                : c.state === "ESTABLISHED"
                                  ? "text-emerald-400"
                                  : "text-slate-500"
                            }
                          >
                            {c.state}
                          </span>
                        </td>
                        <td className="py-1 text-slate-400">
                          {c.process || (c.pid !== "0" ? `PID ${c.pid}` : t("netpan.system"))}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
