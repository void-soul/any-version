import { useState, useEffect, useCallback, useRef } from "react";
import { theamedAlert, alertError, theamedConfirm } from "../shared/ThemedAlert";
import { toast } from "../shared/Toast";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ShieldCheck, Plus, Trash2, Download, Copy, Check } from "lucide-react";
import { CERT_ISSUE_LOG_EVENT, daysUntil, fmtDate, formatIssueResultSummary, issuingSet, type CertDetail, type Certificate, type Credential, type DeployNode, type IssueLogPayload, type IssueResult } from "./CertManager.model";
import { CertForm } from "./CertManager.CertForm";
import { IssueLogModal } from "./CertManager.IssueLogModal";
import { DeployModal } from "./CertManager.DeployModal";
// ---- 证书列表 ----
export function CertList() {
  const { t } = useTranslation();
  const [certs, setCerts] = useState<Certificate[]>([]);
  const [creds, setCreds] = useState<Credential[]>([]);
  const [nodes, setNodes] = useState<DeployNode[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [editing, setEditing] = useState<Certificate | null>(null);
  const [issuing, setIssuing] = useState<Set<string>>(() => new Set(issuingSet));
  const markIssuing = (id: string, on: boolean) => {
    if (on) issuingSet.add(id); else issuingSet.delete(id);
    setIssuing(new Set(issuingSet));
  };
  // 「查看证书」弹窗：详情 + PEM 原文 + 下载证书包
  const [detail, setDetail] = useState<CertDetail | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const [copiedName, setCopiedName] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  // 申请日志：live 是正在跑的那一次（实时流），history 是手动打开的历史
  const [liveLog, setLiveLog] = useState<{
    id: string;
    domain: string;
    lines: string[];
    summary?: string;
  } | null>(null);
  const [historyLog, setHistoryLog] = useState<{ domain: string; lines: string[] } | null>(null);
  /** 正在为哪个证书打开「部署」弹窗 */
  const [deployFor, setDeployFor] = useState<Certificate | null>(null);
  // 日志面板是否还开着：用户中途关掉后失败才需要额外弹一个阻塞提示
  const liveLogOpenRef = useRef(false);

  const refresh = useCallback(async () => {
    setCerts(await invoke<Certificate[]>("cert_list"));
    setCreds(await invoke<Credential[]>("credential_list"));
    setNodes(await invoke<DeployNode[]>("deploy_node_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const issue = async (id: string) => {
    if (issuing.has(id)) return;
    const target = certs.find((c) => c.id === id);
    const threshold = target?.renew_before_days ?? 30;
    const left = daysUntil(target?.not_after);
    // 手动「申请」= 强制重签。证书还没到续期阈值时先说清代价：
    // 同一组域名对 Let's Encrypt 每周只有 5 次重复签发额度，白跑几次当天就签不出来了。
    if (target?.status === "issued" && left !== null && left > threshold) {
      const ok = await theamedConfirm(
        t("certmgr.forceRenewConfirm", { days: left, threshold }),
        { danger: true, confirmText: t("certmgr.apply") },
      );
      if (!ok) return;
    }
    const domain = target?.domains.join(", ") || id;
    markIssuing(id, true);
    liveLogOpenRef.current = true;
    setLiveLog({ id, domain, lines: [] });
    let unlisten: (() => void) | undefined;
    try {
      // 先订阅再 invoke：否则 lego 最早的几行（命令行、账号复用）会在订阅前就推完
      unlisten = await listen<IssueLogPayload>(CERT_ISSUE_LOG_EVENT, (e) => {
        if (e.payload.cert_id !== id) return;
        const line = e.payload.line;
        setLiveLog((prev) =>
          prev && prev.id === id ? { ...prev, lines: [...prev.lines, line] } : prev,
        );
      });
      const res = await invoke<IssueResult>("cert_issue_now", { id });
      await refresh();
      if (res.renewed || (res.deployment?.fail_count ?? 0) > 0) {
        const summary = formatIssueResultSummary(res, t("certmgr.applyDone", { time: fmtDate(res.cert?.not_after) }));
        toast(summary);
        setLiveLog((p) => (p && p.id === id ? { ...p, summary } : p));
      } else {
        // 后端如实回报「没签」：说清楚原因，别让用户以为按钮坏了
        const summary = t("certmgr.applySkipped", { days: res.days_left ?? 0, threshold });
        setLiveLog((p) => (p && p.id === id ? { ...p, summary } : p));
      }
    } catch (e) {
      const msg = String(e);
      setLiveLog((p) =>
        p && p.id === id
          ? { ...p, lines: [...p.lines, "", `✗ ${msg}`], summary: t("certmgr.applyFail", { err: msg }) }
          : p,
      );
      // 日志面板还开着就已经把错误摆在最上面了；只有用户中途关掉时才额外弹提示
      if (!liveLogOpenRef.current) alertError(t("certmgr.applyFail", { err: msg }));
    } finally {
      unlisten?.();
      markIssuing(id, false);
    }
  };

  const closeLiveLog = () => {
    liveLogOpenRef.current = false;
    setLiveLog(null);
  };

  /** 打开某证书的历史申请日志（后端落盘在 certs/logs/<id>.log） */
  const openLog = async (c: Certificate) => {
    const domain = c.domains.join(", ") || c.id;
    try {
      const text = await invoke<string>("cert_issue_log", { id: c.id });
      setHistoryLog({ domain, lines: text ? text.replace(/\n+$/, "").split("\n") : [] });
    } catch (e) {
      theamedAlert(String(e), "err");
    }
  };
  const del = async (id: string) => {
    if (!confirm(t("certmgr.delCertConfirm"))) return;
    await invoke("cert_delete", { id });
    refresh();
  };
  const viewDetail = async (id: string) => {
    try {
      setDetailId(id);
      setDetail(await invoke<CertDetail>("cert_detail", { id }));
    } catch (e) {
      setDetailId(null);
      theamedAlert(String(e));
    }
  };
  const closeDetail = () => {
    setDetail(null);
    setDetailId(null);
    setCopiedName(null);
  };
  const copyPem = async (name: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedName(name);
      setTimeout(() => setCopiedName(null), 1500);
    } catch {
      // 复制失败不打断查看
    }
  };
  // 下载证书包：通用命名 4 文件 + lego 原生文件（后端打包为 zip）
  const downloadZip = async () => {
    if (!detailId) return;
    const domain = certs.find((c) => c.id === detailId)?.domains[0] ?? detailId;
    const safe = domain.replace(/[^A-Za-z0-9._-]/g, "_");
    try {
      const filePath = await saveDialog({
        title: t("certmgr.downloadTitle"),
        defaultPath: `${safe}.zip`,
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (!filePath || typeof filePath !== "string") return;
      setDownloading(true);
      const saved = await invoke<string>("cert_export_zip", {
        id: detailId,
        targetPath: filePath,
      });
      theamedAlert(t("certmgr.downloadDone", { path: saved }));
    } catch (e) {
      alertError(t("certmgr.downloadFail", { err: String(e) }));
    } finally {
      setDownloading(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">{t("certmgr.certsTitle")}</h3>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5"
        >
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newCert")}
        </button>
      </div>

      {showForm && (
        <CertForm
          creds={creds}
          nodes={nodes}
          onDone={() => {
            setShowForm(false);
            refresh();
          }}
        />
      )}
      {editing && (
        <CertForm
          key={editing.id}
          creds={creds}
          nodes={nodes}
          initial={editing}
          onDone={() => {
            setEditing(null);
            refresh();
          }}
        />
      )}

      <div className="overflow-x-auto rounded-ctl border border-white/5">
        <table className="w-full text-caption text-slate-300">
          <thead className="bg-white/5 text-slate-400">
            <tr>
              <th className="px-3 py-2 text-left">{t("certmgr.colDomain")}</th>
              <th className="px-3 py-2 text-left">CA</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colDns")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.validation")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colExpire")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colStatus")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colNodes")}</th>
              <th className="px-3 py-2 text-right">{t("certmgr.colOps")}</th>
            </tr>
          </thead>
          <tbody>
            {certs.map((c) => (
              <tr key={c.id} className="border-t border-white/5">
                <td className="px-3 py-2">{c.domains.join(", ")}</td>
                <td className="px-3 py-2">{c.ca}</td>
                <td className="px-3 py-2">{c.dns_provider || "-"}</td>
                <td className="px-3 py-2">
                  {c.validation === "http"
                    ? t("certmgr.validationHttp")
                    : t("certmgr.validationDns")}
                </td>
                <td className="px-3 py-2">{fmtDate(c.not_after)}</td>
                <td className="px-3 py-2">
                  <span
                    title={c.last_error || undefined}
                    className={
                      c.status === "issued"
                        ? "text-emerald-400"
                        : c.status === "error"
                          ? "text-rose-400"
                          : "text-amber-400"
                    }
                  >
                    {c.status}
                  </span>
                </td>
                <td className="px-3 py-2">
                  {c.deploy_links?.length ? (
                    <span
                      className="text-tiny text-slate-300"
                      title={c.deploy_links
                        .map((l) => {
                          const n = nodes.find((x) => x.id === l.node_id);
                          return `${n?.name ?? l.node_id}${l.target ? ` → ${l.target}` : ""}`;
                        })
                        .join("\n")}
                    >
                      {c.deploy_links
                        .map((l) => {
                          const n = nodes.find((x) => x.id === l.node_id);
                          return `${n?.name ?? l.node_id}${l.target ? `→${l.target}` : ""}`;
                        })
                        .join(" · ")}
                    </span>
                  ) : (
                    <span className="text-slate-500">{t("certmgr.nodesCount", { count: 0 })}</span>
                  )}
                </td>
                <td className="px-3 py-2 text-right whitespace-nowrap">
                  <button onClick={() => issue(c.id)} disabled={issuing.has(c.id)} className="text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] mr-2 disabled:opacity-50">
                    {issuing.has(c.id) ? t("certmgr.issuing") : t("certmgr.apply")}
                  </button>
                  {/* 只要签发过就能部署：即使最近一次续期失败（status=error），
                      磁盘上的旧证书仍然可用，不该拦着 */}
                  <button
                    onClick={() => setDeployFor(c)}
                    disabled={!c.last_issue_at}
                    title={c.last_issue_at ? t("certmgr.deploy") : t("certmgr.notIssued")}
                    className="text-emerald-400 hover:text-emerald-300 mr-2 disabled:opacity-40 disabled:cursor-not-allowed"
                  >
                    {t("certmgr.deploy")}
                  </button>
                  <button
                    onClick={() => { setEditing(c); setShowForm(false); }}
                    className="text-slate-300 hover:text-white mr-2"
                  >
                    {t("certmgr.edit")}
                  </button>
                  <button
                    onClick={() => viewDetail(c.id)}
                    disabled={c.status !== "issued"}
                    title={c.status === "issued" ? t("certmgr.view") : t("certmgr.notIssued")}
                    className="text-sky-400 hover:text-sky-300 mr-2 disabled:opacity-40 disabled:cursor-not-allowed"
                  >
                    {t("certmgr.view")}
                  </button>
                  <button
                    onClick={() => openLog(c)}
                    className="text-slate-400 hover:text-slate-200 mr-2"
                    title={t("certmgr.issueLogHint")}
                  >
                    {t("certmgr.log")}
                  </button>
                  <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300">
                    <Trash2 className="w-3.5 h-3.5 inline" />
                  </button>
                </td>
              </tr>
            ))}
            {certs.length === 0 && (
              <tr>
                <td colSpan={8} className="px-3 py-6 text-center text-slate-500">{t("certmgr.certsEmpty")}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {detail && (
        <div className="fixed inset-0 modal-mask bg-black/60 flex items-center justify-center z-50">
          {/* 面板用 ui-modal（实色背景）：之前误用了 ui-input，5% 白底半透明，后面的列表会透上来 */}
          <div
            className="ui-modal w-[70vw] max-h-[85vh] flex flex-col overflow-hidden"
            onClick={(e) => e.stopPropagation()}
          >
            {/* 头部：标题 + 下载 + 关闭 */}
            <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
              <ShieldCheck className="w-4 h-4 text-[var(--module-accent)] flex-shrink-0" />
              <span className="text-body font-bold text-slate-200 flex-shrink-0">
                {t("certmgr.detailTitle")}
              </span>
              <span className="text-caption text-slate-400 truncate">
                {certs.find((c) => c.id === detailId)?.domains.join(", ")}
              </span>
              <div className="flex-1" />
              <button
                onClick={downloadZip}
                disabled={downloading}
                className="px-2.5 py-1.5 rounded-md bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5 disabled:opacity-50 cursor-pointer"
              >
                <Download className="w-3.5 h-3.5" />
                {downloading ? t("certmgr.downloading") : t("certmgr.download")}
              </button>
              <button
                onClick={closeDetail}
                className="text-caption text-slate-400 hover:text-slate-200 cursor-pointer"
              >
                {t("certmgr.close")}
              </button>
            </div>

            <div className="flex-1 min-h-0 overflow-y-auto px-4 py-3 space-y-3">
              {/* 证书信息 */}
              <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-caption">
                <span className="text-slate-500">{t("certmgr.detailSubject")}</span>
                <span className="text-slate-200 break-all">{detail.subject ?? "—"}</span>
                <span className="text-slate-500">{t("certmgr.detailIssuer")}</span>
                <span className="text-slate-200 break-all">{detail.issuer ?? "—"}</span>
                <span className="text-slate-500">{t("certmgr.detailSerial")}</span>
                <span className="text-slate-200 font-mono break-all">{detail.serial ?? "—"}</span>
                <span className="text-slate-500">{t("certmgr.detailValidity")}</span>
                <span className="text-slate-200">
                  {fmtDate(detail.notBefore)} → {fmtDate(detail.notAfter)}
                  {typeof detail.daysLeft === "number" && (
                    <span
                      className={`ml-2 ${
                        detail.daysLeft < 0
                          ? "text-rose-400"
                          : detail.daysLeft <= 15
                            ? "text-amber-400"
                            : "text-emerald-400"
                      }`}
                    >
                      {detail.daysLeft < 0
                        ? t("certmgr.detailExpired", { days: Math.abs(detail.daysLeft) })
                        : t("certmgr.detailDaysLeft", { days: detail.daysLeft })}
                    </span>
                  )}
                </span>
                <span className="text-slate-500">{t("certmgr.detailSan")}</span>
                <span className="text-slate-200 break-all">
                  {detail.san.length > 0 ? detail.san.join(", ") : "—"}
                </span>
              </div>

              {/* PEM 原文（可逐段复制） */}
              <div className="flex items-center gap-2 pt-1">
                <span className="text-caption font-bold text-slate-300">{t("certmgr.pemTitle")}</span>
                <span className="text-tiny text-slate-500">{t("certmgr.pemHint")}</span>
              </div>
              {Object.keys(detail.pems).length === 0 && (
                <div className="text-caption text-slate-500">{t("certmgr.pemEmpty")}</div>
              )}
              {Object.entries(detail.pems).map(([name, content]) => (
                <div
                  key={name}
                  className="rounded-ctl border border-white/10 bg-black/20 overflow-hidden"
                >
                  <div className="flex items-center gap-2 px-2.5 py-1.5 bg-white/5">
                    <span className="text-tiny text-slate-300 font-mono">{name}</span>
                    <div className="flex-1" />
                    <button
                      onClick={() => copyPem(name, content)}
                      className="text-tiny text-slate-400 hover:text-slate-200 flex items-center gap-1 cursor-pointer"
                    >
                      {copiedName === name ? (
                        <>
                          <Check className="w-3 h-3 text-emerald-400" />
                          {t("certmgr.copied")}
                        </>
                      ) : (
                        <>
                          <Copy className="w-3 h-3" />
                          {t("certmgr.copy")}
                        </>
                      )}
                    </button>
                  </div>
                  <pre className="text-micro p-2 overflow-auto max-h-32 whitespace-pre-wrap text-slate-400">
                    {content}
                  </pre>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* 申请过程实时日志（关掉面板不影响后端继续跑，内容同时落盘） */}
      <IssueLogModal
        open={!!liveLog}
        title={t("certmgr.issueLogTitle", { domain: liveLog?.domain ?? "" })}
        lines={liveLog?.lines ?? []}
        summary={liveLog ? liveLog.summary ?? t("certmgr.issueLogRunning") : undefined}
        onClose={closeLiveLog}
      />
      {/* 历史日志（certs/logs/<证书>.log） */}
      <IssueLogModal
        open={!!historyLog}
        title={t("certmgr.issueLogTitle", { domain: historyLog?.domain ?? "" })}
        lines={historyLog?.lines ?? []}
        onClose={() => setHistoryLog(null)}
      />

      {/* 随时部署：选部署方式 → 逐个推送 → 实时日志 + 结果 + 部署记录 */}
      {deployFor && (
        <DeployModal
          cert={deployFor}
          nodes={nodes}
          onClose={() => setDeployFor(null)}
          onDone={refresh}
        />
      )}
    </div>
  );
}
