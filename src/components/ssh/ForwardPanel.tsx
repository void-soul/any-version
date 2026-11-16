import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Play, Square, X } from "lucide-react";
import { confirmHostKey } from "./hostKeyPrompt";
import { reportInvokeError } from "../../utils/invokeOrReport";

type Kind = "local" | "remote" | "dynamic";

interface ForwardRule {
  id: string; kind: Kind; bindHost: string; bindPort: number; targetHost: string; targetPort: number;
}

const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-tiny border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";
const field = "w-full rounded border border-white/10 bg-black/30 px-2 py-1.5 text-tiny text-slate-100 outline-none focus:border-sky-500/60";

const emptyRule = (): ForwardRule => ({
  id: "", kind: "local", bindHost: "127.0.0.1", bindPort: 8080, targetHost: "", targetPort: 0,
});

/** 单条连接的端口转发管理（本地 / 远程 / SOCKS5 动态）。 */
export function ForwardPanel({ connectionId, title, onClose }: {
  connectionId: string; title: string; onClose: () => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<ForwardRule>(emptyRule);
  const [active, setActive] = useState<ForwardRule[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    try { setActive(await invoke<ForwardRule[]>("ssh_forward_list", { connectionId })); }
    catch (e) { setError(reportInvokeError(e, "ssh_forward_list")); }
  }, [connectionId]);

  useEffect(() => { void refresh(); }, [refresh]);

  const patch = (key: keyof ForwardRule, value: string | number) =>
    setDraft((prev) => ({ ...prev, [key]: value }));

  const start = async () => {
    setBusy(true);
    setError("");
    // 同一毫秒内连续启动多条转发时 Date.now() 会重复，改用 UUID 保证唯一
    const id = `fwd_${crypto.randomUUID()}`;
    const rule = { ...draft, id };
    const launch = async () => invoke<number>("ssh_forward_start", { id, connectionId, rule });
    try {
      let port: number;
      try {
        port = await launch();
      } catch (first) {
        if (!(await confirmHostKey(String(first), (msg) => window.confirm(msg)))) throw first;
        port = await launch();
      }
      setDraft((prev) => ({ ...prev, bindPort: port }));
      await refresh();
    } catch (e) { setError(reportInvokeError(e, "ssh_forward_start")); }
    finally { setBusy(false); }
  };

  const stop = async (id: string) => {
    try {
      await invoke("ssh_forward_stop", { id });
      await refresh();
    } catch (e) { setError(reportInvokeError(e, "ssh_forward_stop")); }
  };

  const dynamic = draft.kind === "dynamic";

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 text-slate-200">
      <div className="flex items-center gap-2">
        <span className="text-tiny font-medium">{title}</span>
        <button className={`${btn} ml-auto`} onClick={onClose}><X className="h-3.5 w-3.5" />{t("ssh.close")}</button>
      </div>

      <div className="grid grid-cols-2 gap-2 rounded border border-white/10 p-3 md:grid-cols-4">
        <label className="block text-tiny text-slate-400">{t("forward.kind")}
          <select className={field} value={draft.kind} onChange={(e) => patch("kind", e.target.value)}>
            <option value="local">{t("forward.local")}</option>
            <option value="remote">{t("forward.remote")}</option>
            <option value="dynamic">{t("forward.dynamic")}</option>
          </select>
        </label>
        <label className="block text-tiny text-slate-400">{t("forward.bindHost")}
          <input className={field} value={draft.bindHost} onChange={(e) => patch("bindHost", e.target.value)} />
        </label>
        <label className="block text-tiny text-slate-400">{t("forward.bindPort")}
          <input className={field} type="number" value={draft.bindPort} onChange={(e) => patch("bindPort", Number(e.target.value))} />
        </label>
        <label className="block text-tiny text-slate-400">{t("forward.targetHost")}
          <input className={field} disabled={dynamic} value={draft.targetHost} onChange={(e) => patch("targetHost", e.target.value)} placeholder={dynamic ? t("forward.dynamicHint") : ""} />
        </label>
        <label className="block text-tiny text-slate-400">{t("forward.targetPort")}
          <input className={field} type="number" disabled={dynamic} value={draft.targetPort} onChange={(e) => patch("targetPort", Number(e.target.value))} />
        </label>
        <button className={`${btn} self-end`} disabled={busy} onClick={() => void start()}>
          <Play className="h-3.5 w-3.5" />{t("forward.start")}
        </button>
      </div>

      {error && <div className="rounded border border-rose-400/30 bg-rose-400/10 px-3 py-2 text-tiny text-rose-200">{error}</div>}

      <div className="min-h-0 flex-1 overflow-auto rounded border border-white/10">
        <table className="w-full text-tiny">
          <thead className="bg-slate-900/90 text-slate-400">
            <tr>
              <th className="px-3 py-2 text-left">{t("forward.kind")}</th>
              <th className="px-3 py-2 text-left">{t("forward.listen")}</th>
              <th className="px-3 py-2 text-left">{t("forward.target")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.actions")}</th>
            </tr>
          </thead>
          <tbody>
            {active.length === 0 && <tr><td colSpan={4} className="px-3 py-8 text-center text-slate-500">{t("forward.empty")}</td></tr>}
            {active.map((r) => (
              <tr key={r.id} className="border-t border-white/5">
                <td className="px-3 py-2">{t(`forward.${r.kind}`)}</td>
                <td className="px-3 py-2">{r.bindHost}:{r.bindPort}</td>
                <td className="px-3 py-2 text-slate-400">{r.kind === "dynamic" ? "SOCKS5" : `${r.targetHost}:${r.targetPort}`}</td>
                <td className="px-3 py-2">
                  <button className={btn} onClick={() => void stop(r.id)}><Square className="h-3 w-3" />{t("forward.stop")}</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
