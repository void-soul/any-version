import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { toast } from "../shared/Toast";
import { alertError } from "../shared/ThemedAlert";

/** 与后端 `CertReceiverConfig` 对齐（字段名 = 旧 cert-receiver.toml 的键名） */
interface CertReceiverConfig {
  listen_addr: string;
  token: string;
  output_dir: string;
  layout: string;
  api_key: string;
  reload_mode: string;
  reload_command: string;
  nginx_prefix: string;
  nginx_conf: string;
  service_name: string;
  post_script?: string | null;
  auto_start: boolean;
}

interface ReceiverStatus {
  running: boolean;
  addr: string;
  pushes: number;
  fails: number;
  last_push_at: string;
  last_push_ok: boolean;
  last_error: string;
}

interface StoredCert {
  dir: string;
  certificate_id: string;
  domains: string[];
  expired_time: string;
  updated_at_ms: number;
}

interface Bootstrap {
  config: CertReceiverConfig;
  status: ReceiverStatus;
  logs: string[];
  migrated: boolean;
}

const LOG_EVENT = "cert-receiver-log";
const STATUS_EVENT = "cert-receiver-status";

/** 生成 32 位十六进制（token / api_key） */
function randomHex(bytes: number): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, "0")).join("");
}

const label = "text-tiny text-slate-400";
const input = "ui-input w-full px-2 py-1.5 text-tiny";

/**
 * 接收端页签：配置 / 启停 / 已收到证书 / 拉取示例 / 实时日志。
 *
 * 后端是 `commands::cert_receiver`（原独立小工具 cert-receiver 的集成版），
 * 接口与落盘语义完全沿用，老部署脚本不用改。
 */
export default function CertReceiverView() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<CertReceiverConfig | null>(null);
  const [status, setStatus] = useState<ReceiverStatus | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [stored, setStored] = useState<StoredCert[]>([]);
  const [busy, setBusy] = useState(false);
  const [showToken, setShowToken] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  const [apiId, setApiId] = useState("kira-pull");
  const [certId, setCertId] = useState("");
  const [curl, setCurl] = useState("");
  const [note, setNote] = useState("");
  const logRef = useRef<HTMLDivElement | null>(null);

  const loadStored = useCallback(async () => {
    try {
      setStored(await invoke<StoredCert[]>("cert_receiver_stored_certs"));
    } catch {
      setStored([]);
    }
  }, []);

  // 首次加载：配置 / 状态 / 历史日志 + 订阅实时事件
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const boot = await invoke<Bootstrap>("cert_receiver_bootstrap");
        if (!alive) return;
        setCfg(boot.config);
        setStatus(boot.status);
        setLogs(boot.logs);
        if (boot.migrated) {
          toast(t("certmgr.rcvMigrated"));
        }
        await loadStored();
      } catch (e) {
        alertError(String(e));
      }
    })();

    const unlisteners: Array<() => void> = [];
    listen<string>(LOG_EVENT, (e) => {
      setLogs((prev) => [...prev.slice(-499), e.payload]);
    }).then((un) => unlisteners.push(un));
    listen<ReceiverStatus>(STATUS_EVENT, (e) => {
      setStatus(e.payload);
      loadStored();
    }).then((un) => unlisteners.push(un));

    return () => {
      alive = false;
      unlisteners.forEach((un) => un());
    };
  }, [loadStored, t]);

  // 日志贴底
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs.length]);

  // 拉取示例：证书列表变化或 api_key 变化时重算
  useEffect(() => {
    if (!certId && stored.length > 0) setCertId(stored[0].certificate_id);
  }, [stored, certId]);

  useEffect(() => {
    (async () => {
      if (!certId) {
        setCurl("");
        setNote(t("certmgr.rcvPullEmpty"));
        return;
      }
      try {
        const ex = await invoke<{ url: string; curl: string; timestamp: string }>(
          "cert_receiver_pull_example",
          { apiId, certificateId: certId },
        );
        setCurl(ex.curl);
        setNote(
          cfg?.listen_addr?.startsWith("0.0.0.0")
            ? t("certmgr.rcvPullNoteAny")
            : t("certmgr.rcvPullNote"),
        );
      } catch (e) {
        setCurl("");
        setNote(String(e));
      }
    })();
  }, [apiId, certId, cfg?.listen_addr, cfg?.api_key, t]);

  const patch = (p: Partial<CertReceiverConfig>) =>
    setCfg((prev) => (prev ? { ...prev, ...p } : prev));

  const save = async () => {
    if (!cfg) return;
    setBusy(true);
    try {
      const st = await invoke<ReceiverStatus>("cert_receiver_save_config", { config: cfg });
      setStatus(st);
      toast(t("certmgr.rcvSaved"));
    } catch (e) {
      alertError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = async () => {
    setBusy(true);
    try {
      const cmd = status?.running ? "cert_receiver_stop" : "cert_receiver_start";
      const st = await invoke<ReceiverStatus>(cmd);
      setStatus(st);
    } catch (e) {
      alertError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const pickDir = async () => {
    if (!cfg) return;
    try {
      const picked = await openDialog({
        directory: true,
        defaultPath: cfg.output_dir || undefined,
      });
      if (typeof picked === "string" && picked) patch({ output_dir: picked });
    } catch (e) {
      alertError(String(e));
    }
  };

  const openDir = async (rel?: string) => {
    if (!cfg) return;
    const base = cfg.output_dir.replace(/[\\/]+$/, "");
    const target = rel ? `${base}\\${rel}` : base;
    try {
      await openPath(target);
    } catch (e) {
      alertError(String(e));
    }
  };

  if (!cfg) {
    return <div className="text-caption text-slate-500 p-2">{t("certmgr.rcvLoading")}</div>;
  }

  const running = !!status?.running;

  return (
    <div className="space-y-3">
      {/* ① 状态条 */}
      <div className="flex items-center gap-3 rounded-card border border-white/10 bg-white/5 px-3 py-2">
        <span
          className={`inline-flex items-center gap-1.5 text-caption font-semibold ${
            running ? "text-emerald-400" : "text-slate-400"
          }`}
        >
          <span
            className={`w-2 h-2 rounded-full ${running ? "bg-emerald-400" : "bg-slate-500"}`}
          />
          {running ? t("certmgr.rcvRunning") : t("certmgr.rcvStopped")}
        </span>
        <span className="text-tiny text-slate-400">{status?.addr || cfg.listen_addr}</span>
        <span className="text-tiny text-slate-500">
          {t("certmgr.rcvStats", {
            ok: status?.pushes ?? 0,
            fail: status?.fails ?? 0,
          })}
          {status?.last_push_at ? ` · ${status.last_push_at}` : ""}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <button
            type="button"
            onClick={toggle}
            disabled={busy}
            className="px-3 py-1 rounded-ctl bg-white/10 hover:bg-white/15 text-caption text-slate-200 disabled:opacity-50 cursor-pointer"
          >
            {running ? t("certmgr.rcvStop") : t("certmgr.rcvStart")}
          </button>
          <button
            type="button"
            onClick={() => openDir()}
            className="px-3 py-1 rounded-ctl bg-white/10 hover:bg-white/15 text-caption text-slate-200 cursor-pointer"
          >
            {t("certmgr.rcvOpenDir")}
          </button>
        </div>
      </div>

      {status?.last_error && !status.last_push_ok && (
        <div className="text-tiny text-rose-400 break-all">{status.last_error}</div>
      )}

      {/* ② 服务配置 */}
      <div className="rounded-card border border-white/10 bg-white/5 p-3 space-y-2">
        <div className="grid grid-cols-2 gap-2">
          <label className={label}>
            {t("certmgr.rcvListenAddr")}
            <select
              className={input}
              value={cfg.listen_addr.split(":")[0]}
              onChange={(e) => {
                const port = cfg.listen_addr.split(":").slice(1).join(":") || "9000";
                patch({ listen_addr: `${e.target.value}:${port}` });
              }}
            >
              <option value="127.0.0.1">127.0.0.1 — 仅本机</option>
              <option value="0.0.0.0">0.0.0.0 — 允许其它机器 / 容器访问</option>
            </select>
          </label>
          <label className={label}>
            {t("certmgr.rcvPort")}
            <input
              className={input}
              value={cfg.listen_addr.split(":").slice(1).join(":")}
              onChange={(e) => {
                const host = cfg.listen_addr.split(":")[0] || "127.0.0.1";
                patch({ listen_addr: `${host}:${e.target.value.replace(/[^0-9]/g, "")}` });
              }}
            />
          </label>
          <label className={label}>
            {t("certmgr.rcvLayout")}
            <select
              className={input}
              value={cfg.layout}
              onChange={(e) => patch({ layout: e.target.value })}
            >
              <option value="cert">cert — 按证书 ID 建目录</option>
              <option value="domain">domain — 按域名建目录</option>
              <option value="both">both — 两份都写</option>
              <option value="flat">flat — 平铺 .crt / .key</option>
            </select>
          </label>
          <label className={label}>
            {t("certmgr.rcvReloadMode")}
            <select
              className={input}
              value={cfg.reload_mode}
              onChange={(e) => patch({ reload_mode: e.target.value })}
            >
              <option value="none">none — 不重载</option>
              <option value="nginx">nginx — -t 后 reload</option>
              <option value="command">command — 执行命令</option>
              <option value="service">service — 重启服务</option>
            </select>
          </label>
        </div>

        <label className={label}>
          {t("certmgr.rcvOutputDir")}
          <span className="flex items-center gap-2">
            <input
              className={input}
              value={cfg.output_dir}
              onChange={(e) => patch({ output_dir: e.target.value })}
            />
            <button
              type="button"
              onClick={pickDir}
              className="shrink-0 px-2 py-1 rounded-ctl bg-white/10 hover:bg-white/15 text-tiny text-slate-200 cursor-pointer"
            >
              {t("certmgr.rcvChoose")}
            </button>
          </span>
        </label>

        <div className="grid grid-cols-2 gap-2">
          <label className={label}>
            {t("certmgr.rcvToken")}
            <span className="flex items-center gap-1.5">
              <input
                className={input}
                type={showToken ? "text" : "password"}
                value={cfg.token}
                onChange={(e) => patch({ token: e.target.value })}
              />
              <button
                type="button"
                onClick={() => patch({ token: randomHex(16) })}
                className="shrink-0 px-2 py-1 rounded-ctl bg-white/10 text-tiny text-slate-200 cursor-pointer"
              >
                {t("certmgr.rcvGenerate")}
              </button>
              <button
                type="button"
                onClick={() => setShowToken((v) => !v)}
                className="shrink-0 px-2 py-1 rounded-ctl bg-white/10 text-tiny text-slate-200 cursor-pointer"
              >
                {showToken ? t("certmgr.rcvHide") : t("certmgr.rcvShow")}
              </button>
            </span>
          </label>
          <label className={label}>
            {t("certmgr.rcvApiKey")}
            <span className="flex items-center gap-1.5">
              <input
                className={input}
                type={showApiKey ? "text" : "password"}
                value={cfg.api_key}
                onChange={(e) => patch({ api_key: e.target.value })}
              />
              <button
                type="button"
                onClick={() => patch({ api_key: randomHex(16) })}
                className="shrink-0 px-2 py-1 rounded-ctl bg-white/10 text-tiny text-slate-200 cursor-pointer"
              >
                {t("certmgr.rcvGenerate")}
              </button>
              <button
                type="button"
                onClick={() => setShowApiKey((v) => !v)}
                className="shrink-0 px-2 py-1 rounded-ctl bg-white/10 text-tiny text-slate-200 cursor-pointer"
              >
                {showApiKey ? t("certmgr.rcvHide") : t("certmgr.rcvShow")}
              </button>
            </span>
          </label>
        </div>

        {cfg.reload_mode === "nginx" && (
          <div className="grid grid-cols-2 gap-2">
            <label className={label}>
              {t("certmgr.rcvNginxPrefix")}
              <input
                className={input}
                value={cfg.nginx_prefix}
                placeholder="D:\\nginx"
                onChange={(e) => patch({ nginx_prefix: e.target.value })}
              />
            </label>
            <label className={label}>
              {t("certmgr.rcvNginxConf")}
              <input
                className={input}
                value={cfg.nginx_conf}
                placeholder="D:\\nginx\\conf\\nginx.conf"
                onChange={(e) => patch({ nginx_conf: e.target.value })}
              />
            </label>
          </div>
        )}
        {cfg.reload_mode === "command" && (
          <label className={label}>
            {t("certmgr.rcvReloadCommand")}
            <input
              className={input}
              value={cfg.reload_command}
              onChange={(e) => patch({ reload_command: e.target.value })}
            />
          </label>
        )}
        {cfg.reload_mode === "service" && (
          <label className={label}>
            {t("certmgr.rcvServiceName")}
            <input
              className={input}
              value={cfg.service_name}
              onChange={(e) => patch({ service_name: e.target.value })}
            />
          </label>
        )}

        <div className="flex items-center gap-3 pt-1">
          <button
            type="button"
            onClick={save}
            disabled={busy}
            className="px-4 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold disabled:opacity-50 cursor-pointer"
          >
            {t("certmgr.rcvSaveApply")}
          </button>
          <label className="flex items-center gap-1.5 text-tiny text-slate-400 cursor-pointer">
            <input
              type="checkbox"
              checked={cfg.auto_start}
              onChange={(e) => patch({ auto_start: e.target.checked })}
            />
            {t("certmgr.rcvAutoStart")}
          </label>
        </div>
      </div>

      {/* ③ 已收到证书 */}
      <div className="rounded-card border border-white/10 bg-white/5 p-3">
        <div className="flex items-center gap-2 mb-2">
          <span className="text-caption font-bold text-slate-300">{t("certmgr.rcvStored")}</span>
          <span className="text-tiny text-slate-500">{stored.length}</span>
          <button
            type="button"
            onClick={loadStored}
            className="ml-auto px-2 py-0.5 rounded-ctl bg-white/10 text-tiny text-slate-300 cursor-pointer"
          >
            {t("certmgr.rcvRefresh")}
          </button>
        </div>
        {stored.length === 0 ? (
          <div className="text-tiny text-slate-500">{t("certmgr.rcvStoredEmpty")}</div>
        ) : (
          <div className="max-h-48 overflow-y-auto">
            <table className="w-full text-tiny">
              <thead className="text-slate-500">
                <tr>
                  <th className="text-left font-normal py-1">{t("certmgr.rcvColId")}</th>
                  <th className="text-left font-normal py-1">{t("certmgr.rcvColDomains")}</th>
                  <th className="text-left font-normal py-1">{t("certmgr.rcvColExpire")}</th>
                  <th className="text-left font-normal py-1">{t("certmgr.rcvColDir")}</th>
                  <th className="w-16" />
                </tr>
              </thead>
              <tbody className="text-slate-300">
                {stored.map((c) => (
                  <tr key={c.dir} className="border-t border-white/5">
                    <td className="py-1 pr-2 break-all">{c.certificate_id}</td>
                    <td className="py-1 pr-2 break-all">{c.domains.join(", ")}</td>
                    <td className="py-1 pr-2 whitespace-nowrap">
                      {(c.expired_time || "").slice(0, 10) || "-"}
                    </td>
                    <td className="py-1 pr-2 break-all text-slate-400">{c.dir}</td>
                    <td className="py-1 text-right">
                      <button
                        type="button"
                        onClick={() => openDir(c.dir)}
                        className="text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] cursor-pointer"
                      >
                        {t("certmgr.rcvOpen")}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* ④ 拉取示例 */}
      <div className="rounded-card border border-white/10 bg-white/5 p-3 space-y-2">
        <div className="text-caption font-bold text-slate-300">{t("certmgr.rcvPullTitle")}</div>
        <div className="grid grid-cols-2 gap-2">
          <label className={label}>
            apiId
            <input className={input} value={apiId} onChange={(e) => setApiId(e.target.value)} />
          </label>
          <label className={label}>
            {t("certmgr.rcvPullCert")}
            <select
              className={input}
              value={certId}
              onChange={(e) => setCertId(e.target.value)}
            >
              {stored.length === 0 && <option value="">{t("certmgr.rcvStoredEmpty")}</option>}
              {stored.map((c) => (
                <option key={c.dir} value={c.certificate_id}>
                  {c.certificate_id}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label className={label}>
          curl
          <span className="flex items-center gap-2">
            <input className={input} readOnly value={curl} />
            <button
              type="button"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(curl);
                  toast(t("certmgr.rcvCopied"));
                } catch {
                  alertError(t("certmgr.rcvCopyFail"));
                }
              }}
              className="shrink-0 px-2 py-1 rounded-ctl bg-white/10 text-tiny text-slate-200 cursor-pointer"
            >
              {t("certmgr.rcvCopy")}
            </button>
          </span>
        </label>
        <div className="text-micro text-slate-500">{note}</div>
      </div>

      {/* ⑤ 接收日志 */}
      <div className="rounded-card border border-white/10 bg-white/5 p-3">
        <div className="flex items-center gap-2 mb-2">
          <span className="text-caption font-bold text-slate-300">{t("certmgr.rcvLog")}</span>
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              onClick={async () => {
                await invoke("cert_receiver_clear_logs");
                setLogs([]);
              }}
              className="px-2 py-0.5 rounded-ctl bg-white/10 text-tiny text-slate-300 cursor-pointer"
            >
              {t("certmgr.rcvClear")}
            </button>
          </div>
        </div>
        <div
          ref={logRef}
          className="max-h-56 overflow-y-auto rounded-ctl border border-white/10 bg-black/40 p-2"
        >
          {logs.length === 0 ? (
            <div className="text-tiny text-slate-500">{t("certmgr.rcvLogEmpty")}</div>
          ) : (
            <pre className="text-micro font-mono whitespace-pre-wrap break-all text-slate-300">
              {logs.join("\n")}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
