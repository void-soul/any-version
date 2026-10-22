// Sub-Store 页 —— 下载 / 启动 / 停止 / 打开面板 / 订阅与组合列表
// 后端 substore.rs 已实现完整的下载、node 后端托管与静态前端服务，
// 但此前只有 mihomoApi 封装，界面上没有任何入口。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Boxes, Download, ExternalLink, FolderOpen, Play, RefreshCw, Save, Square } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { Toggle, btnPrimary, btnSec, cardCls, inputCls, labelCls, tagCls } from "./ui";

type SubStoreStatus = {
  downloaded?: boolean;
  frontendReady?: boolean;
  running?: boolean;
  backendPort?: number;
  frontendPort?: number;
  url?: string;
  dir?: string;
};

function asList(v: any): any[] {
  if (Array.isArray(v)) return v;
  if (Array.isArray(v?.data)) return v.data;
  return [];
}
function itemName(v: any): string {
  return String(v?.name || v?.displayName || v?.url || "-");
}

export default function SubStorePanel({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const [st, setSt] = useState<SubStoreStatus>({});
  const [busy, setBusy] = useState("");
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  const [subs, setSubs] = useState<any[]>([]);
  const [cols, setCols] = useState<any[]>([]);
  const [backendPort, setBackendPort] = useState("38324");
  const [frontendPort, setFrontendPort] = useState("38325");
  const [useProxy, setUseProxy] = useState(false);

  const flash = (s: string) => { setMsg(s); setErr(""); };
  const fail = (s: string) => { setErr(s); setMsg(""); };

  const refreshStatus = async () => {
    try {
      const s: SubStoreStatus = await mihomoApi.substoreStatus();
      setSt(s || {});
      return s;
    } catch (e: any) {
      fail(String(e));
      return null;
    }
  };

  const loadCfg = async () => {
    try {
      const a: any = await mihomoApi.getAppConfig();
      if (a?.subStoreBackendPort) setBackendPort(String(a.subStoreBackendPort));
      if (a?.subStoreFrontendPort) setFrontendPort(String(a.subStoreFrontendPort));
      setUseProxy(!!a?.useProxyInSubStore);
    } catch (e: any) {
      fail(String(e));
    }
  };

  const loadLists = async () => {
    setBusy("list");
    try {
      const [s, c] = await Promise.all([
        mihomoApi.substoreSubs().catch(() => null),
        mihomoApi.substoreCollections().catch(() => null),
      ]);
      setSubs(asList(s));
      setCols(asList(c));
      flash(t("mihomosubstore.refreshOk"));
    } catch (e: any) {
      fail(t("mihomosubstore.listFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  useEffect(() => {
    loadCfg();
    refreshStatus().then((s) => { if (s?.running) loadLists(); });
  }, []);

  const saveCfg = async () => {
    setBusy("save");
    try {
      const b = Number(backendPort) || 38324;
      const f = Number(frontendPort) || 38325;
      await mihomoApi.patchAppConfig({
        subStoreBackendPort: b,
        subStoreFrontendPort: f,
        useProxyInSubStore: useProxy,
      });
      flash(t("mihomosubstore.saved"));
    } catch (e: any) {
      fail(t("mihomosubstore.saveFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doDownload = async () => {
    setBusy("download");
    try {
      await mihomoApi.substoreDownload();
      await refreshStatus();
      flash(t("mihomosubstore.downloadOk"));
    } catch (e: any) {
      fail(t("mihomosubstore.downloadFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doStart = async () => {
    setBusy("start");
    try {
      const r: any = await mihomoApi.substoreStart();
      await refreshStatus();
      await loadLists();
      flash(t("mihomosubstore.startOk", { url: r?.url || "" }));
    } catch (e: any) {
      fail(t("mihomosubstore.startFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doStop = async () => {
    setBusy("stop");
    try {
      await mihomoApi.substoreStop();
      await refreshStatus();
      flash(t("mihomosubstore.stopOk"));
    } catch (e: any) {
      fail(String(e));
    } finally {
      setBusy("");
    }
  };

  const openPanel = async () => {
    if (!st.url) return;
    try { await openUrl(st.url); }
    catch (e: any) { fail(String(e)); }
  };

  return (
    <div className="space-y-3">
      {msg && <div className="text-caption text-emerald-300 px-1 break-all">{msg}</div>}
      {err && <div className="text-caption text-rose-300 px-1 break-all">{err}</div>}

      {/* 状态与生命周期 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center gap-2 mb-1">
          <Boxes className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-sm font-bold text-white">{t("mihomosubstore.title")}</h3>
          <span className={tagCls}>{st.downloaded ? t("mihomosubstore.downloaded") : t("mihomosubstore.notDownloaded")}</span>
          <span className={`${tagCls} ${st.running ? "!text-emerald-300" : ""}`}>
            {st.running ? t("mihomosubstore.running") : t("mihomosubstore.stopped")}
          </span>
        </div>
        <p className="text-caption text-slate-400 mb-2">{t("mihomosubstore.hint")}</p>

        {st.url && (
          <div className="text-caption text-slate-300 font-mono break-all mb-2">
            {t("mihomosubstore.url")}: {st.url}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <button className={btnSec} onClick={doDownload} disabled={busy === "download"}>
            <span className="inline-flex items-center gap-1.5">
              <Download className={`w-3.5 h-3.5 ${busy === "download" ? "animate-bounce" : ""}`} />
              {busy === "download" ? t("mihomosubstore.downloading") : t("mihomosubstore.download")}
            </span>
          </button>
          {!st.running ? (
            <button className={btnPrimary} onClick={doStart} disabled={busy === "start" || !st.downloaded}>
              <span className="inline-flex items-center gap-1.5">
                <Play className="w-3.5 h-3.5 fill-current" />
                {busy === "start" ? t("mihomosubstore.starting") : t("mihomosubstore.start")}
              </span>
            </button>
          ) : (
            <button className={btnSec} onClick={doStop} disabled={busy === "stop"}>
              <span className="inline-flex items-center gap-1.5">
                <Square className="w-3.5 h-3.5 fill-current" />
                {busy === "stop" ? t("mihomosubstore.stopping") : t("mihomosubstore.stop")}
              </span>
            </button>
          )}
          <button className={btnSec} onClick={openPanel} disabled={!st.running || !st.url}>
            <span className="inline-flex items-center gap-1.5">
              <ExternalLink className="w-3.5 h-3.5" />{t("mihomosubstore.open")}
            </span>
          </button>
          <button
            className={btnSec}
            onClick={() => st.dir && mihomoApi.openPath("substore").catch((e) => fail(String(e)))}
            disabled={!st.dir}
            title={st.dir || ""}
          >
            <span className="inline-flex items-center gap-1.5">
              <FolderOpen className="w-3.5 h-3.5" />{t("mihomosubstore.openDir")}
            </span>
          </button>
        </div>
        {!st.downloaded && (
          <div className="mt-2 text-tiny text-amber-300">{t("mihomosubstore.downloadFirst")}</div>
        )}
      </div>

      {/* 端口与代理设置 */}
      <div className={`${cardCls} p-4`}>
        <h3 className="text-sm font-bold text-white mb-2">{t("mihomosubstore.settingsTitle")}</h3>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className={labelCls}>{t("mihomosubstore.backendPort")}</label>
            <input className={inputCls} type="number" min={1} max={65535} value={backendPort}
              onChange={(e) => setBackendPort(e.target.value)} />
          </div>
          <div>
            <label className={labelCls}>{t("mihomosubstore.frontendPort")}</label>
            <input className={inputCls} type="number" min={1} max={65535} value={frontendPort}
              onChange={(e) => setFrontendPort(e.target.value)} />
          </div>
        </div>
        <div className="mt-2">
          <Toggle label={t("mihomosubstore.useProxy")} v={useProxy} onChange={setUseProxy} />
        </div>
        <div className="mt-3 flex items-center gap-2">
          <button className={btnPrimary} onClick={saveCfg} disabled={busy === "save"}>
            <span className="inline-flex items-center gap-1.5">
              <Save className="w-3.5 h-3.5" />{t("mihomosubstore.save")}
            </span>
          </button>
          <span className="text-tiny text-slate-500">{t("mihomosubstore.restartHint")}</span>
        </div>
      </div>

      {/* 订阅 / 组合列表 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-bold text-white">{t("mihomosubstore.listsTitle")}</h3>
          <button className={btnSec} onClick={loadLists} disabled={busy === "list" || !st.running}>
            <span className="inline-flex items-center gap-1.5">
              <RefreshCw className={`w-3.5 h-3.5 ${busy === "list" ? "animate-spin" : ""}`} />
              {t("mihomosubstore.refresh")}
            </span>
          </button>
        </div>
        {!st.running && <div className="text-caption text-slate-500">{t("mihomosubstore.notRunningHint")}</div>}
        {st.running && (
          <div className="grid grid-cols-2 gap-3">
            <div>
              <div className="text-caption text-slate-400 font-semibold mb-1">
                {t("mihomosubstore.subsTitle")} <span className={tagCls}>{subs.length}</span>
              </div>
              <div className="rounded-card border border-white/10 divide-y divide-white/5 max-h-56 overflow-y-auto">
                {subs.length === 0 && <div className="px-3 py-2 text-caption text-slate-500">{t("mihomosubstore.empty")}</div>}
                {subs.map((s, i) => (
                  <div key={`${itemName(s)}-${i}`} className="px-3 py-1.5 text-caption text-slate-200 truncate" title={itemName(s)}>
                    {itemName(s)}
                  </div>
                ))}
              </div>
            </div>
            <div>
              <div className="text-caption text-slate-400 font-semibold mb-1">
                {t("mihomosubstore.collectionsTitle")} <span className={tagCls}>{cols.length}</span>
              </div>
              <div className="rounded-card border border-white/10 divide-y divide-white/5 max-h-56 overflow-y-auto">
                {cols.length === 0 && <div className="px-3 py-2 text-caption text-slate-500">{t("mihomosubstore.empty")}</div>}
                {cols.map((c, i) => (
                  <div key={`${itemName(c)}-${i}`} className="px-3 py-1.5 text-caption text-slate-200 truncate" title={itemName(c)}>
                    {itemName(c)}
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
      </div>

      {/* 代理未运行时给出明确提示（启动 Sub-Store 不依赖内核，但订阅解析会用到） */}
      {!running && st.running && (
        <div className="text-tiny text-slate-500 px-1">{t("mihomosubstore.coreStoppedHint")}</div>
      )}
    </div>
  );
}
