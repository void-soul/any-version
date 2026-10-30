import { useState, useEffect, useCallback, useRef } from "react";
import { alertError } from "../shared/ThemedAlert";
import { toast } from "../shared/Toast";
import { SharedModal } from "../shared/Modal";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { CERT_ISSUE_LOG_EVENT, NODE_TYPE_LABELS, fmtDate, type Certificate, type DeployNode, type DeployRecord, type DeploySummary, type IssueLogPayload } from "./CertManager.model";
/**
 * 部署弹窗：随时把已签发的证书推给任意部署方式。
 *
 * 与「申请」分开：只推送 + 重载，不重新签发。日志复用同一个 `cert-issue-log`
 * 事件与同一份日志文件，所以这里的实时输出和「申请」时看到的是同一套东西。
 */
export function DeployModal({
  cert,
  nodes,
  onClose,
  onDone,
}: {
  cert: Certificate;
  nodes: DeployNode[];
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  // 默认勾选证书已关联的部署方式；也可以临时改（改动不回写证书配置）
  const [checked, setChecked] = useState<Record<string, string>>(() => {
    const src = cert.deploy_links?.length
      ? cert.deploy_links
      : (cert.deploy_node_ids ?? []).map((node_id) => ({ node_id, target: "" }));
    return Object.fromEntries(src.map((l) => [l.node_id, l.target]));
  });
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const [summary, setSummary] = useState<DeploySummary | null>(null);
  const [history, setHistory] = useState<DeployRecord[]>([]);
  const boxRef = useRef<HTMLDivElement | null>(null);

  const loadHistory = useCallback(async () => {
    try {
      setHistory(await invoke<DeployRecord[]>("deploy_history", { id: cert.id }));
    } catch {
      setHistory([]);
    }
  }, [cert.id]);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  useEffect(() => {
    const el = boxRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length]);

  const start = async () => {
    const links = Object.entries(checked).map(([node_id, target]) => ({ node_id, target }));
    if (links.length === 0) return;
    setRunning(true);
    setSummary(null);
    setLines([]);
    let unlisten: (() => void) | undefined;
    try {
      // 先订阅再调用，避免漏掉最早几行
      unlisten = await listen<IssueLogPayload>(CERT_ISSUE_LOG_EVENT, (e) => {
        if (e.payload.cert_id !== cert.id) return;
        const line = e.payload.line;
        setLines((prev) => [...prev, line]);
      });
      const res = await invoke<DeploySummary>("cert_deploy_now", { id: cert.id, links });
      setSummary(res);
      toast(
        t("certmgr.deployResult", { ok: res.ok_count, fail: res.fail_count }),
        res.fail_count > 0 ? "err" : "ok",
      );
      onDone();
      await loadHistory();
    } catch (e) {
      const msg = String(e);
      setLines((prev) => [...prev, "", `✗ ${msg}`]);
      alertError(msg);
    } finally {
      unlisten?.();
      setRunning(false);
    }
  };

  return (
    <SharedModal
      open
      onClose={onClose}
      title={t("certmgr.deployTitle", { domain: cert.domains.join(", ") })}
      width={820}
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption cursor-pointer"
          >
            {t("certmgr.close")}
          </button>
          <button
            type="button"
            onClick={start}
            disabled={running || Object.keys(checked).length === 0}
            className="px-4 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold disabled:opacity-50 cursor-pointer"
          >
            {running ? t("certmgr.deployRunning") : t("certmgr.deployStart")}
          </button>
        </div>
      }
    >
      <div className="space-y-3">
        <p className="text-tiny text-slate-500">{t("certmgr.deployHint")}</p>

        <div className="max-h-40 overflow-y-auto space-y-1.5">
          {nodes.map((n) => {
            const on = checked[n.id] !== undefined;
            return (
              <div
                key={n.id}
                className="flex items-center gap-2 text-tiny text-slate-300 bg-black/20 rounded px-2 py-1.5"
              >
                <label className="flex items-center gap-1.5 cursor-pointer shrink-0">
                  <input
                    type="checkbox"
                    checked={on}
                    onChange={(e) =>
                      setChecked((prev) => {
                        const next = { ...prev };
                        if (e.target.checked) next[n.id] = next[n.id] ?? "";
                        else delete next[n.id];
                        return next;
                      })
                    }
                  />
                  {n.name}
                  <span className="text-micro text-slate-500">
                    {NODE_TYPE_LABELS[n.type] ?? n.type}
                  </span>
                </label>
                {on && (
                  <input
                    value={checked[n.id]}
                    onChange={(e) => setChecked((prev) => ({ ...prev, [n.id]: e.target.value }))}
                    className="ml-auto w-56 bg-black/30 rounded px-2 py-1 text-tiny text-slate-200 outline-none"
                    placeholder={t("certmgr.targetPh")}
                  />
                )}
              </div>
            );
          })}
          {nodes.length === 0 && (
            <div className="text-caption text-slate-500">{t("certmgr.deployNoNodes")}</div>
          )}
        </div>

        {summary && (
          <div className="space-y-1 rounded-ctl border border-white/10 bg-white/5 p-2">
            {summary.results.map((r) => (
              <div key={r.node_id} className="text-tiny flex items-center gap-2 min-w-0">
                <span className={r.ok ? "text-emerald-400" : "text-rose-400"}>
                  {r.ok ? "✓" : "✗"}
                </span>
                <span className="text-slate-300 shrink-0">{r.node_name}</span>
                {r.target && <span className="text-slate-500 shrink-0">→ {r.target}</span>}
                <span
                  className={r.ok ? "text-slate-500 truncate" : "text-rose-400 truncate"}
                  title={r.message}
                >
                  {r.message}
                </span>
              </div>
            ))}
          </div>
        )}

        <div>
          <div className="flex items-center gap-2 mb-1">
            <span className="text-caption font-bold text-slate-300">{t("certmgr.deployLog")}</span>
            <span className="text-tiny text-slate-500">
              {running ? t("certmgr.deployRunning") : ""}
            </span>
          </div>
          <div
            ref={boxRef}
            className="max-h-44 overflow-y-auto rounded-ctl border border-white/10 bg-black/40 p-2"
          >
            {lines.length === 0 ? (
              <div className="text-caption text-slate-500">{t("certmgr.deployLogEmpty")}</div>
            ) : (
              <pre className="text-micro font-mono whitespace-pre-wrap break-all text-slate-300">
                {lines.join("\n")}
              </pre>
            )}
          </div>
        </div>

        <details className="rounded-ctl border border-white/10 bg-white/5">
          <summary className="text-caption text-slate-300 px-2.5 py-1.5 cursor-pointer select-none">
            {t("certmgr.deployHistory")}（{history.length}）
          </summary>
          <div className="max-h-44 overflow-y-auto px-2.5 pb-2 space-y-1">
            {history.length === 0 && (
              <div className="text-tiny text-slate-500">{t("certmgr.deployHistoryEmpty")}</div>
            )}
            {history.map((h, i) => (
              <div key={`${h.at}-${h.node_id}-${i}`} className="text-tiny flex items-center gap-2">
                <span className="text-slate-500 shrink-0">{fmtDate(h.at)}</span>
                <span className={h.ok ? "text-emerald-400" : "text-rose-400"}>
                  {h.ok ? "✓" : "✗"}
                </span>
                <span className="text-slate-300 shrink-0">{h.node_name}</span>
                {h.target && <span className="text-slate-500 shrink-0">→ {h.target}</span>}
                <span className="text-slate-500 truncate" title={h.message}>
                  {h.message}
                </span>
                <span className="ml-auto text-micro text-slate-600 shrink-0">
                  {t(`certmgr.deployTrigger_${h.trigger}`)}
                </span>
              </div>
            ))}
          </div>
        </details>
      </div>
    </SharedModal>
  );
}
