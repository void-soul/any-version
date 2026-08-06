// 订阅页（对齐 clash-party profiles.tsx + profile-item + edit-info-modal）
import React, { useEffect, useMemo, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import {
  Download, FolderOpen, FilePlus2, RefreshCw, Trash2, Pencil, FileCode2,
  ChevronUp, ChevronDown, CheckCircle2, Link2, Clipboard, ScrollText,
} from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { mihomoApi } from "../mihomoApi";
import {
  inputCls, labelCls, btnSec, btnDanger, btnPrimary, tagCls, cardCls,
  Toggle, Modal, calcTraffic,
} from "./ui";

type ProfileItem = {
  id: string;
  name: string;
  type: string;
  url?: string | null;
  auth_token?: string | null;
  user_agent?: string | null;
  age_secret_key?: string | null;
  use_proxy: boolean;
  auto_update: boolean;
  update_interval: number;
  update_timeout: number;
  override_ids: string[];
  subscription_userinfo?: {
    upload: number; download: number; total: number; expire: number;
  } | null;
  updated_at?: number | null;
  [k: string]: any;
};

const fmtTime = (sec?: number | null) => {
  if (!sec) return "从未更新";
  const d = new Date(sec * 1000);
  const diff = Date.now() / 1000 - sec;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return d.toLocaleString("zh-CN", { hour12: false });
};

const fmtExpire = (sec?: number) => {
  if (!sec) return "长期有效";
  return new Date(sec * 1000).toLocaleDateString("zh-CN");
};

// 订阅请求日志条目：记录每次导入/更新/自动更新的关键请求信息
type SubLog = {
  id: number;
  time: number;
  type: string;     // 导入 / 更新 / 更新全部 / 自动更新 / 校验
  name: string;     // 订阅名或地址
  proxy: boolean;   // 是否走代理更新
  ok: boolean;
  msg?: string;     // 失败原因或备注
  used?: number;
  total?: number;
  remaining?: number;
  expire?: number;
};

export default function SubscriptionsPanel({
  running,
  onNavigate,
}: {
  running?: boolean;
  onNavigate?: (tab: string) => void;
}) {
  const [cfg, setCfg] = useState<{ current: string; items: ProfileItem[] }>({
    current: "default",
    items: [],
  });
  const [overrides, setOverrides] = useState<any[]>([]);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string>("");
  const [msg, setMsg] = useState<string>("");
  const [status, setStatus] = useState<Record<string, { available: boolean; node_count: number }>>({});
  const [editInfo, setEditInfo] = useState<ProfileItem | null>(null);
  const [editFile, setEditFile] = useState<{ item: ProfileItem; content: string } | null>(null);
  const [liveSubs, setLiveSubs] = useState<any>(null);
  const [logs, setLogs] = useState<SubLog[]>([]);
  const [showLog, setShowLog] = useState(false);
  const logSeq = useRef(0);
  const timerRef = useRef<any>(null);

  // 追加一条订阅请求日志（最多保留 100 条，最新在前）
  const pushLog = (l: Omit<SubLog, "id" | "time">) => {
    setLogs((prev) => [{ id: ++logSeq.current, time: Date.now(), ...l }, ...prev].slice(0, 100));
  };

  // 内核在跑时，实时拉取 /subscriptions 余额（核心已注入 subscription-userinfo），
  // 优先用于"当前配置"的卡片展示，刚更新/切换后即可立即看到余额
  useEffect(() => {
    if (!running) { setLiveSubs(null); return; }
    let alive = true;
    const fetchLive = async () => {
      try {
        const r = await mihomoApi.api("GET", "/subscriptions");
        if (alive && r && typeof r === "object") setLiveSubs(r);
      } catch {}
    };
    fetchLive();
    const t = setInterval(fetchLive, 30_000);
    return () => { alive = false; clearInterval(t); };
  }, [running]);

  const uiOf = (it: ProfileItem) => {
    if (running && it.id === cfg.current && liveSubs && (liveSubs.total || liveSubs.expire)) {
      return {
        upload: liveSubs.upload || 0,
        download: liveSubs.download || 0,
        total: liveSubs.total || 0,
        expire: liveSubs.expire || 0,
      };
    }
    return it.subscription_userinfo;
  };

  const load = async () => {
    try {
      const c = await mihomoApi.getProfileConfig();
      // 当前选中项以 app_config.current_profile 为准（托盘也是读它）：
      // ProfileConfig.current 默认恒为 "default"，change_current_profile 不会更新它，
      // 故这里从 getAppConfig 取真实值覆盖，否则绿色边框永远无法命中。
      let cur = c.current;
      try {
        const ac = await mihomoApi.getAppConfig();
        if (ac && typeof ac.current_profile === "string" && ac.current_profile) {
          cur = ac.current_profile;
        }
      } catch {}
      setCfg({ current: cur, items: c.items || [] });
      const o = await mihomoApi.getOverrideConfig().catch(() => ({ items: [] }));
      setOverrides(o?.items || []);
      // 拉取各订阅的可用性/节点数
      const st: Record<string, any> = {};
      await Promise.all(
        (c.items || []).map(async (it: ProfileItem) => {
          try { st[it.id] = await mihomoApi.getProfileStatus(it.id); } catch {}
        })
      );
      setStatus(st);
    } catch (e: any) {
      setMsg(String(e));
    }
  };

  useEffect(() => { load(); }, []);

  // 定时更新由后端调度器负责（对齐 clash-party profileUpdater 在主进程执行），
  // 界面只负责定期同步列表状态，避免关闭面板后不再更新、以及前后端重复下载
  useEffect(() => {
    clearInterval(timerRef.current);
    timerRef.current = setInterval(() => { load(); }, 60_000);
    return () => clearInterval(timerRef.current);
  }, []);

  const flash = (s: string) => { setMsg(s); setTimeout(() => setMsg(""), 3000); };

  const doImport = async () => {
    const u = url.trim();
    if (!u) return;
    setBusy("import");
    try {
      const v = await mihomoApi.validateSubscription(u);
      if (v && v.ok === false) {
        pushLog({ type: "校验", name: u, proxy: false, ok: false, msg: v.message });
        if (!confirm(`订阅校验提示：${v.message}\n仍要导入吗？`)) { setBusy(""); return; }
      } else if (v && v.message) {
        pushLog({ type: "校验", name: u, proxy: false, ok: true, msg: v.message });
      }
      const imported: any = await mihomoApi.importSubscription(u);
      const ui = imported?.subscription_userinfo;
      pushLog({
        type: "导入",
        name: u,
        proxy: !!imported?.use_proxy,
        ok: true,
        used: ui ? ui.upload + ui.download : undefined,
        total: ui?.total,
        remaining: ui ? Math.max(0, ui.total - (ui.upload + ui.download)) : undefined,
        expire: ui?.expire,
      });
      setUrl("");
      await load();
      flash("导入成功");
    } catch (e: any) {
      pushLog({ type: "导入", name: u, proxy: false, ok: false, msg: String(e) });
      flash(`导入失败: ${e}`);
    }
    setBusy("");
  };

  const pasteUrl = async () => {
    try { setUrl((await navigator.clipboard.readText()).trim()); } catch {}
  };

  const doOpenFile = async () => {
    const p = await openDialog({
      multiple: false,
      filters: [{ name: "Clash 配置", extensions: ["yaml", "yml"] }],
    });
    if (!p || typeof p !== "string") return;
    setBusy("file");
    try { await mihomoApi.importFile(p); await load(); flash("导入成功"); }
    catch (e: any) { flash(`导入失败: ${e}`); }
    setBusy("");
  };

  const doNewBlank = async () => {
    const name = prompt("新建配置名称", "新建配置");
    if (!name) return;
    const id = `local_${Date.now()}`;
    try {
      await mihomoApi.addProfile({
        id, name, type: "file", subscriptions: [], rule_providers: [],
        custom_rules: [], dns_enabled: false, dns_nameservers: [],
        use_proxy: false, auto_update: false, update_interval: 86400,
        update_timeout: 30, override_ids: [],
      });
      await mihomoApi.setProfileStr(id, "proxies: []\nproxy-groups: []\nrules: []\n");
      await load();
      flash("已创建");
    } catch (e: any) { flash(`创建失败: ${e}`); }
  };

  const doUpdate = async (it: ProfileItem) => {
    if (!it.url) { flash("该配置没有订阅地址"); return; }
    setBusy(it.id);
    try {
      const updated: any = await mihomoApi.updateSubscription(it.id);
      if (cfg.current === it.id) await mihomoApi.updateRuntimeConfig();
      const ui = updated?.subscription_userinfo;
      pushLog({
        type: "更新",
        name: it.name,
        proxy: it.use_proxy,
        ok: true,
        used: ui ? ui.upload + ui.download : undefined,
        total: ui?.total,
        remaining: ui ? Math.max(0, ui.total - (ui.upload + ui.download)) : undefined,
        expire: ui?.expire,
      });
      await load();
      flash("更新成功");
    } catch (e: any) {
      pushLog({ type: "更新", name: it.name, proxy: it.use_proxy, ok: false, msg: String(e) });
      flash(`更新失败: ${e}`);
    }
    setBusy("");
  };

  const doUpdateAll = async () => {
    setBusy("all");
    let ok = 0, fail = 0;
    for (const it of cfg.items) {
      if (!it.url) continue;
      try {
        const updated: any = await mihomoApi.updateSubscription(it.id);
        const ui = updated?.subscription_userinfo;
        pushLog({
          type: "更新",
          name: it.name,
          proxy: it.use_proxy,
          ok: true,
          used: ui ? ui.upload + ui.download : undefined,
          total: ui?.total,
          remaining: ui ? Math.max(0, ui.total - (ui.upload + ui.download)) : undefined,
          expire: ui?.expire,
        });
        ok++;
      } catch (e: any) {
        pushLog({ type: "更新", name: it.name, proxy: it.use_proxy, ok: false, msg: String(e) });
        fail++;
      }
    }
    try { await mihomoApi.updateRuntimeConfig(); } catch {}
    await load();
    setBusy("");
    flash(`更新完成：成功 ${ok}，失败 ${fail}`);
    pushLog({
      type: "更新全部",
      name: `${cfg.items.filter((i) => i.url).length} 个订阅`,
      proxy: false,
      ok: fail === 0,
      msg: `成功 ${ok}，失败 ${fail}`,
    });
  };

  const doSelect = async (it: ProfileItem) => {
    if (cfg.current === it.id) { onNavigate?.("proxies"); return; }
    setBusy(it.id);
    try {
      await mihomoApi.changeCurrentProfile(it.id);
      // 通知代理/规则页立即按新订阅刷新（核心已在后台 reload）
      window.dispatchEvent(new CustomEvent("mihomo:profile-changed", { detail: { id: it.id } }));
      await load();
      flash(`已切换到 ${it.name}`);
      onNavigate?.("proxies"); // 跳到代理页，直观看到该订阅的节点
    }
    catch (e: any) { flash(`切换失败: ${e}`); }
    setBusy("");
  };

  const doRemove = async (it: ProfileItem) => {
    if (!confirm(`确认删除配置「${it.name}」？`)) return;
    try { await mihomoApi.removeProfile(it.id); await load(); }
    catch (e: any) { flash(`删除失败: ${e}`); }
  };

  const move = async (idx: number, dir: -1 | 1) => {
    const items = [...cfg.items];
    const j = idx + dir;
    if (j < 0 || j >= items.length) return;
    [items[idx], items[j]] = [items[j], items[idx]];
    setCfg({ ...cfg, items });
    try { await mihomoApi.setProfileConfig({ ...cfg, items }); } catch {}
  };

  const openEditFile = async (it: ProfileItem) => {
    try { setEditFile({ item: it, content: await mihomoApi.getProfileStr(it.id) }); }
    catch (e: any) { flash(`读取失败: ${e}`); }
  };

  const openInExplorer = async (it: ProfileItem) => {
    try { await openPath(await mihomoApi.getProfileFilePath(it.id)); }
    catch (e: any) { flash(`打开失败: ${e}`); }
  };

  return (
    <div className="space-y-4">
      {/* 导入栏 */}
      <div className={`${cardCls} p-3`}>
        <div className="flex items-center gap-2">
          <div className="relative flex-1">
            <Link2 className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-500" />
            <input
              className={`${inputCls} pl-8`}
              placeholder="输入或粘贴订阅链接，回车导入"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && doImport()}
            />
          </div>
          <button className={btnSec} onClick={pasteUrl} title="从剪贴板粘贴">
            <Clipboard className="w-3.5 h-3.5" />
          </button>
          <button className={btnPrimary} onClick={doImport} disabled={busy === "import"}>
            <span className="flex items-center gap-1.5">
              <Download className="w-3.5 h-3.5" />
              {busy === "import" ? "导入中…" : "导入"}
            </span>
          </button>
          <button className={btnSec} onClick={doOpenFile} title="打开本地 yaml">
            <span className="flex items-center gap-1.5"><FolderOpen className="w-3.5 h-3.5" /> 本地文件</span>
          </button>
          <button className={btnSec} onClick={doNewBlank} title="新建空白配置">
            <span className="flex items-center gap-1.5"><FilePlus2 className="w-3.5 h-3.5" /> 新建</span>
          </button>
          <button className={btnSec} onClick={doUpdateAll} disabled={busy === "all"}>
            <span className="flex items-center gap-1.5">
              <RefreshCw className={`w-3.5 h-3.5 ${busy === "all" ? "animate-spin" : ""}`} /> 更新全部
            </span>
          </button>
        </div>
        {msg && <div className="mt-2 text-[11px] text-emerald-300">{msg}</div>}
      </div>

      {/* 配置卡片列表 */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
        {cfg.items.map((it, idx) => {
          const cur = cfg.current === it.id;
          const ui = uiOf(it);
          const used = ui ? ui.upload + ui.download : 0;
          const pct = ui && ui.total > 0 ? Math.min(100, (used / ui.total) * 100) : 0;
          const st = status[it.id];
          return (
            <div
              key={it.id}
              onClick={() => doSelect(it)}
              className={`${cardCls} p-3 cursor-pointer transition-all ${
                cur ? "border-emerald-400 ring-1 ring-emerald-400/30 bg-emerald-500/[0.06]" : "hover:border-white/20"
              }`}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5">
                    {cur && <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />}
                    <span className="text-[13px] font-semibold text-white break-all" title={it.name}>{it.name}</span>
                    <span className={tagCls}>{it.url ? "订阅" : "本地"}</span>
                    {st && (
                      <span className={tagCls}>
                        {st.available ? `${st.node_count} 节点` : "不可用"}
                      </span>
                    )}
                  </div>
                  {it.url && (
                    <div className="text-[10px] text-slate-500 truncate mt-0.5" title={it.url}>
                      {it.url}
                    </div>
                  )}
                </div>
                <div className="flex items-center gap-1 flex-shrink-0" onClick={(e) => e.stopPropagation()}>
                  <button className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
                    title="上移" onClick={() => move(idx, -1)}>
                    <ChevronUp className="w-3.5 h-3.5" />
                  </button>
                  <button className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
                    title="下移" onClick={() => move(idx, 1)}>
                    <ChevronDown className="w-3.5 h-3.5" />
                  </button>
                  {it.url && (
                    <button className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-emerald-300 cursor-pointer"
                      title="更新" onClick={() => doUpdate(it)}>
                      <RefreshCw className={`w-3.5 h-3.5 ${busy === it.id ? "animate-spin" : ""}`} />
                    </button>
                  )}
                  <button className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
                    title="编辑信息" onClick={() => setEditInfo(it)}>
                    <Pencil className="w-3.5 h-3.5" />
                  </button>
                  <button className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
                    title="编辑文件" onClick={() => openEditFile(it)}>
                    <FileCode2 className="w-3.5 h-3.5" />
                  </button>
                  <button className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
                    title="在文件夹中打开" onClick={() => openInExplorer(it)}>
                    <FolderOpen className="w-3.5 h-3.5" />
                  </button>
                  <button className="p-1.5 rounded-lg hover:bg-rose-500/20 text-slate-400 hover:text-rose-300 cursor-pointer"
                    title="删除" onClick={() => doRemove(it)}>
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>

              {/* 流量用量（显式展示本月剩余流量） */}
              {ui && (
                <div className="mt-2.5">
                  <div className="flex items-center justify-between text-[10px] text-slate-400 mb-1">
                    <span>
                      {ui.total > 0 ? (
                        <>
                          剩余{" "}
                          <b className="text-emerald-300 font-semibold">
                            {calcTraffic(Math.max(0, ui.total - used))}
                          </b>{" "}
                          / {calcTraffic(ui.total)}
                        </>
                      ) : (
                        <>已用 {calcTraffic(used)}</>
                      )}
                    </span>
                    <span>到期 {fmtExpire(ui.expire)}</span>
                  </div>
                  {ui.total > 0 && (
                    <div className="h-1.5 rounded-full bg-white/10 overflow-hidden">
                      <div
                        className={`h-full rounded-full ${pct > 90 ? "bg-rose-400" : pct > 70 ? "bg-amber-400" : "bg-emerald-400"}`}
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  )}
                </div>
              )}

              <div className="mt-2 flex items-center justify-between text-[10px] text-slate-500">
                <span>{fmtTime(it.updated_at)}</span>
                <span className="flex items-center gap-2">
                  {it.auto_update && <span className={tagCls}>自动更新 {Math.round((it.update_interval || 0) / 60)}m</span>}
                  {it.use_proxy && <span className={tagCls}>走代理</span>}
                  {(it.override_ids?.length || 0) > 0 && <span className={tagCls}>覆写 {it.override_ids.length}</span>}
                </span>
              </div>
            </div>
          );
        })}
        {cfg.items.length === 0 && (
          <div className="text-center text-xs text-slate-500 py-10 col-span-full">
            暂无配置，粘贴订阅链接导入或打开本地 yaml 文件
          </div>
        )}
      </div>

      {/* 订阅请求日志（导入/更新/自动更新都会记录，含代理与本月剩余流量） */}
      <div className={`${cardCls} p-3`}>
        <div
          className="flex items-center justify-between cursor-pointer select-none"
          onClick={() => setShowLog((v) => !v)}
        >
          <div className="flex items-center gap-2 text-[12px] font-semibold text-white">
            <ScrollText className="w-4 h-4 text-emerald-400" />
            订阅请求日志
            <span className={tagCls}>{logs.length}</span>
          </div>
          <div className="flex items-center gap-3">
            {logs.length > 0 && (
              <button
                className="text-[11px] text-slate-400 hover:text-rose-300"
                onClick={(e) => { e.stopPropagation(); setLogs([]); }}
              >
                清空
              </button>
            )}
            {showLog
              ? <ChevronUp className="w-4 h-4 text-slate-400" />
              : <ChevronDown className="w-4 h-4 text-slate-400" />}
          </div>
        </div>
        {showLog && (
          <div className="mt-3 space-y-1 max-h-64 overflow-y-auto pr-1">
            {logs.length === 0 && (
              <div className="text-[11px] text-slate-500">
                暂无请求记录（导入 / 更新 / 自动更新订阅时会自动记录，含代理与本月剩余流量）
              </div>
            )}
            {logs.map((l) => {
              const border = l.ok ? "border-emerald-500/40" : "border-rose-500/50";
              return (
                <div
                  key={l.id}
                  className={`flex items-center gap-2 text-[11px] border-l-2 pl-2 py-0.5 ${border}`}
                >
                  <span className="text-slate-500 shrink-0">
                    {new Date(l.time).toLocaleTimeString("zh-CN", { hour12: false })}
                  </span>
                  <span className={`px-1.5 py-0.5 rounded shrink-0 ${l.ok ? "bg-emerald-500/15 text-emerald-300" : "bg-rose-500/15 text-rose-300"}`}>
                    {l.type}
                  </span>
                  <span className="text-slate-300 truncate flex-1" title={l.name}>{l.name}</span>
                  {l.proxy && <span className={tagCls}>代理</span>}
                  {typeof l.total === "number" && l.total > 0 ? (
                    <span className="text-slate-400 shrink-0">
                      剩 {calcTraffic(Math.max(0, l.remaining || 0))} / {calcTraffic(l.total)}
                    </span>
                  ) : typeof l.used === "number" ? (
                    <span className="text-slate-400 shrink-0">已用 {calcTraffic(l.used)}</span>
                  ) : null}
                  {l.msg && (
                    <span className="text-rose-300/80 shrink-0 max-w-[40%] truncate" title={l.msg}>{l.msg}</span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {editInfo && (
        <EditInfoModal
          item={editInfo}
          overrides={overrides}
          onClose={() => setEditInfo(null)}
          onSaved={async () => { setEditInfo(null); await load(); flash("已保存"); }}
        />
      )}
      {editFile && (
        <EditFileModal
          item={editFile.item}
          initial={editFile.content}
          isCurrent={cfg.current === editFile.item.id}
          onClose={() => setEditFile(null)}
          onSaved={async () => { setEditFile(null); await load(); flash("已保存"); }}
        />
      )}
    </div>
  );
}

// ---- 编辑信息弹窗（对齐 clash-party edit-info-modal）----
function EditInfoModal({ item, overrides, onClose, onSaved }: any) {
  const [v, setV] = useState<ProfileItem>({ ...item });
  const [saving, setSaving] = useState(false);
  const set = (k: string, val: any) => setV((p) => ({ ...p, [k]: val }));

  const save = async () => {
    setSaving(true);
    try { await mihomoApi.updateProfile(v); onSaved(); }
    catch (e: any) { alert(`保存失败: ${e}`); }
    setSaving(false);
  };

  return (
    <Modal
      title="编辑信息"
      onClose={onClose}
      busy={saving}
      busyText="保存中…"
      footer={
        <>
          <button className={btnSec} onClick={onClose} disabled={saving}>取消</button>
          <button className={btnPrimary} onClick={save} disabled={saving}>{saving ? "保存中…" : "保存"}</button>
        </>
      }
    >
      <div className="grid grid-cols-2 gap-3">
        <div className="col-span-2">
          <label className={labelCls}>名称</label>
          <input className={inputCls} value={v.name} onChange={(e) => set("name", e.target.value)} />
        </div>
        <div className="col-span-2">
          <label className={labelCls}>订阅地址</label>
          <input className={inputCls} value={v.url || ""} placeholder="https://..."
            onChange={(e) => set("url", e.target.value)} />
        </div>
        <div>
          <label className={labelCls}>User Agent</label>
          <input className={inputCls} value={v.user_agent || ""} placeholder="clash-verge/v1.7.0"
            onChange={(e) => set("user_agent", e.target.value)} />
        </div>
        <div>
          <label className={labelCls}>授权令牌</label>
          <input className={inputCls} value={v.auth_token || ""} placeholder="Bearer token"
            onChange={(e) => set("auth_token", e.target.value)} />
        </div>
        <div>
          <label className={labelCls}>更新间隔（分钟）</label>
          <input className={inputCls} type="number" min={1}
            value={Math.round((v.update_interval || 86400) / 60)}
            onChange={(e) => set("update_interval", Math.max(1, Number(e.target.value) || 1) * 60)} />
        </div>
        <div>
          <label className={labelCls}>更新超时（秒）</label>
          <input className={inputCls} type="number" min={5}
            value={v.update_timeout || 30}
            onChange={(e) => set("update_timeout", Math.max(5, Number(e.target.value) || 30))} />
        </div>
        <div className="col-span-2">
          <label className={labelCls}>Age 解密密钥（可选）</label>
          <input className={inputCls} value={v.age_secret_key || ""} placeholder="AGE-SECRET-KEY-..."
            onChange={(e) => set("age_secret_key", e.target.value)} />
        </div>
        <div className="col-span-2 flex items-center gap-6 pt-1">
          <Toggle label="自动更新" v={!!v.auto_update} onChange={(b) => set("auto_update", b)} />
          <Toggle label="使用代理更新" v={!!v.use_proxy} onChange={(b) => set("use_proxy", b)} />
        </div>
        <div className="col-span-2">
          <label className={labelCls}>应用覆写</label>
          <div className="flex flex-wrap gap-2">
            {overrides.length === 0 && <span className="text-[11px] text-slate-500">暂无覆写脚本</span>}
            {overrides.map((o: any) => {
              const on = (v.override_ids || []).includes(o.id);
              return (
                <button
                  key={o.id}
                  className={`px-2.5 py-1 rounded-lg text-[11px] border transition-all cursor-pointer ${
                    on
                      ? "bg-emerald-500/20 text-emerald-300 border-emerald-500/30"
                      : "bg-white/5 text-slate-400 border-white/10 hover:text-white"
                  }`}
                  onClick={() => {
                    const ids = new Set(v.override_ids || []);
                    on ? ids.delete(o.id) : ids.add(o.id);
                    set("override_ids", [...ids]);
                  }}
                >
                  {o.name} <span className="opacity-60">.{o.ext}</span>
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </Modal>
  );
}

// ---- 编辑文件弹窗 ----
function EditFileModal({ item, initial, isCurrent, onClose, onSaved }: any) {
  const [text, setText] = useState(initial);
  const [saving, setSaving] = useState(false);
  const lines = useMemo(() => text.split("\n").length, [text]);

  const save = async () => {
    setSaving(true);
    try {
      await mihomoApi.setProfileStr(item.id, text);
      if (isCurrent) await mihomoApi.updateRuntimeConfig();
      onSaved();
    } catch (e: any) { alert(`保存失败: ${e}`); }
    setSaving(false);
  };

  return (
    <Modal
      title={`编辑文件 - ${item.name}`}
      wide
      onClose={onClose}
      busy={saving}
      busyText="保存并重载配置…"
      footer={
        <>
          <span className="text-[11px] text-slate-500 mr-auto">{lines} 行</span>
          <button className={btnSec} onClick={onClose} disabled={saving}>取消</button>
          <button className={btnPrimary} onClick={save} disabled={saving}>{saving ? "保存中…" : "保存"}</button>
        </>
      }
    >
      <div className="w-full h-[55vh] rounded-xl border border-white/10 overflow-hidden">
        <Editor
          height="55vh"
          language="yaml"
          theme="vs-dark"
          value={text}
          onChange={(v) => setText(v ?? "")}
          options={{ minimap: { enabled: false }, fontSize: 12, scrollBeyondLastLine: false, wordWrap: "on", automaticLayout: true }}
        />
      </div>
    </Modal>
  );
}
