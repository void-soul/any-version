// 二级代理页 —— 管理多个家庭 socks5 二级代理，选择启用哪个连接
// 链路：网络请求 → 一级代理（在「代理」页选中） → 二级代理（本页启用） → 目标
// 界面与「代理」页统一：网格列数可调、选中绿色背景+绿色边框、支持测速
import React, { useEffect, useRef, useState } from "react";
import { Plus, Pencil, Trash2, Zap, RefreshCw, Info, Layers, Check, ChevronDown, Filter, X } from "lucide-react";
import { mihomoApi, SecondaryProxy } from "../mihomoApi";
import { cardCls, Modal, btnSec, btnPrimary, inputCls, labelCls, delayColor, delayText } from "./ui";
import { ctrlGet, proxyDelay, lastDelay } from "./ctrl";

export default function SecondaryProxiesPanel({ running }: { running: boolean }) {
  const [items, setItems] = useState<SecondaryProxy[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [cfg, setCfg] = useState<any>({});
  const [live, setLive] = useState<Record<string, any>>({}); // 节点名 -> /proxies 里的 proxy 数据（含 history）
  const [testing, setTesting] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<SecondaryProxy | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState("");
  const [currentProxy, setCurrentProxy] = useState("");
  const [profileId, setProfileId] = useState("");
  const timerRef = useRef<any>(null);

  const cols: number = Number(cfg?.proxy_cols) || 2;
  const delayUrl: string = cfg?.delayTestUrl || "http://www.gstatic.com/generate_204";
  const delayTimeout: number = Number(cfg?.delayTestTimeout) || 5000;

  const load = async () => {
    try {
      const a = await mihomoApi.getAppConfig();
      setCfg(a || {});
      setItems((a?.secondary_proxies || []).map((s: any) => ({ ...s, port: Number(s.port) || 0 })));
      setActiveId(a?.secondary_active_id || null);
      setCurrentProxy(a?.default_proxy || "");
      try {
        const pc = await mihomoApi.getProfileConfig();
        setProfileId(pc?.current_profile || "");
      } catch {}
    } catch {}
  };
  useEffect(() => { load(); }, [running]);

  // 运行时拉取二级节点实时数据（用于测速显示）
  const fetchLive = async () => {
    if (!running) { setLive({}); return; }
    try {
      const d: any = await ctrlGet("/proxies");
      setLive(d?.proxies || {});
    } catch {}
  };
  useEffect(() => {
    fetchLive();
    clearInterval(timerRef.current);
    timerRef.current = setInterval(fetchLive, 5000);
    return () => clearInterval(timerRef.current);
  }, [running]);

  const genId = () => "s" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
  const nodeName = (s: SecondaryProxy) => `二级-${s.name}`;

  // 保存整份列表 + 启用项
  const persist = async (list: SecondaryProxy[], active: string | null, tip?: string) => {
    setSaving(true);
    setMsg("");
    try {
      await mihomoApi.saveSecondaryProxies(list, active);
      setItems(list);
      setActiveId(active);
      if (tip) setMsg(tip);
    } catch (e: any) { setMsg(String(e)); }
    finally { setSaving(false); }
  };

  // 启用/停用二级代理；id 无效时停用全部（不使用二级代理）
  const toggleActive = (id: string) => {
    const nextActive = activeId === id ? null : id; // 再点一次取消
    persist(items, nextActive, nextActive ? "已启用二级代理" : "已停用二级代理");
  };

  const saveEdit = async (patch: SecondaryProxy) => {
    if (!patch.name.trim()) { setMsg("请填写二级代理名称"); return; }
    if (!patch.host.trim() || !patch.port) { setMsg("请填写 IP/域名 与 端口"); return; }
    let list: SecondaryProxy[];
    if (isNew) {
      list = [...items, { ...patch, id: patch.id || genId() }];
    } else {
      list = items.map((s) => (s.id === patch.id ? patch : s));
    }
    await persist(list, activeId, isNew ? "已新增二级代理" : "已保存修改");
    setEditing(null);
    setIsNew(false);
  };

  const remove = async (id: string) => {
    const list = items.filter((s) => s.id !== id);
    const active = activeId === id ? null : activeId;
    await persist(list, active, "已删除二级代理");
  };

  // 全部测速
  const onTestAll = async () => {
    if (!running || !items.length) return;
    setTesting(Object.fromEntries(items.map((s) => [s.id, true])));
    try {
      await Promise.all(items.map((s) =>
        proxyDelay(nodeName(s), delayUrl, delayTimeout).catch(() => {}).then(() => fetchLive())
      ));
    } finally {
      setTesting({});
      fetchLive();
    }
  };

  // 单节点测速
  const onProxyDelay = async (s: SecondaryProxy) => {
    setTesting((t) => ({ ...t, [s.id]: true }));
    try {
      await proxyDelay(nodeName(s), delayUrl, delayTimeout);
    } catch {}
    setTesting((t) => ({ ...t, [s.id]: false }));
    fetchLive();
  };

  const empty = activeId === null;

  return (
    <div className="space-y-3">
      {/* 顶栏 */}
      <div className="flex items-center gap-2 flex-wrap">
        <div className="flex items-center gap-2 text-xs text-slate-400 min-w-0">
          <Info className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />
          <span className="truncate">链路：一级代理（代理页选中：{currentProxy || "未设置"}） → 二级代理 → 目标</span>
        </div>
        <div className="flex-1" />
        {/* 列数切换（与代理页一致） */}
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] text-slate-400">列数</span>
          <div className="flex items-center rounded-lg bg-white/10 p-0.5">
            {[1, 2, 3, 4].map((c) => (
              <button
                key={c}
                onClick={async () => { await mihomoApi.patchAppConfig({ proxy_cols: c }); load(); }}
                className={`min-w-[22px] h-6 rounded-md text-[11px] font-semibold transition-colors cursor-pointer ${
                  cols === c ? "bg-white/20 text-white" : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {c}
              </button>
            ))}
          </div>
        </div>
        <button className={btnSec} onClick={fetchLive} disabled={!running}>
          <span className="inline-flex items-center gap-1"><RefreshCw className="w-3 h-3" />刷新</span>
        </button>
        <button className={btnSec} onClick={onTestAll} disabled={!running || items.length === 0}>
          <span className="inline-flex items-center gap-1">
            <Zap className={`w-3 h-3 ${Object.keys(testing).length ? "animate-spin" : ""}`} />全部测速
          </span>
        </button>
        <button
          className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-xl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-[12px] font-semibold cursor-pointer transition-all whitespace-nowrap"
          onClick={() => { setEditing({ id: genId(), name: "", host: "", port: 0, username: "", password: "" }); setIsNew(true); }}
        >
          <Plus className="w-4 h-4" /> 新增二级代理
        </button>
      </div>
      {msg && <div className="text-[11px] text-emerald-300 px-1">{msg}</div>}

      {/* 二级代理网格（与代理页一致的列数 + 绿色选中） */}
      <div className={`grid gap-1.5`} style={{ gridTemplateColumns: `repeat(${cols}, minmax(0,1fr))` }}>
        {/* 不使用二级代理 */}
        <div
          onClick={() => toggleActive("")}
          className={`px-2.5 py-2 rounded-xl border cursor-pointer transition-all ${
            empty ? "bg-emerald-500/10 border-emerald-500/40" : "bg-white/[0.03] border-white/5 hover:border-white/20"
          }`}
        >
          <div className="flex items-center gap-1.5">
            <Layers className={`w-3.5 h-3.5 flex-shrink-0 ${empty ? "text-emerald-300" : "text-slate-400"}`} />
            <span className={`text-[12px] truncate flex-1 ${empty ? "text-emerald-300 font-semibold" : "text-slate-200"}`}>
              不使用二级代理
            </span>
          </div>
          <div className="mt-0.5 text-[10px] text-slate-500 truncate">请求直接经一级代理出站</div>
        </div>

        {items.map((s) => {
          const isActive = activeId === s.id;
          const p = live[nodeName(s)];
          const d = lastDelay(p);
          const testingNow = !!testing[s.id];
          return (
            <div
              key={s.id}
              onClick={() => toggleActive(s.id)}
              className={`px-2.5 py-2 rounded-xl border cursor-pointer transition-all ${
                isActive ? "bg-emerald-500/10 border-emerald-500/40" : "bg-white/[0.03] border-white/5 hover:border-white/20"
              }`}
            >
              <div className="flex items-center gap-1.5">
                <Layers className={`w-3.5 h-3.5 flex-shrink-0 ${isActive ? "text-emerald-300" : "text-slate-400"}`} />
                <span className={`text-[12px] truncate flex-1 ${isActive ? "text-emerald-300 font-semibold" : "text-slate-200"}`}>
                  {s.name}
                </span>
                <button
                  className={`text-[10px] font-mono flex-shrink-0 cursor-pointer hover:underline ${delayColor(d)}`}
                  title="测试此二级代理延迟"
                  disabled={testingNow}
                  onClick={(e) => { e.stopPropagation(); onProxyDelay(s); }}
                >
                  {testingNow ? (
                    <span className="inline-block w-3 h-3 rounded-full border border-white/30 border-t-white animate-spin align-middle" />
                  ) : (
                    delayText(d)
                  )}
                </button>
              </div>
              <div className="mt-0.5 text-[10px] text-slate-500 truncate font-mono">
                {s.host}:{s.port}
              </div>
              <div className="mt-1 flex items-center justify-between">
                <span className={`text-[9px] px-1.5 py-0.5 rounded ${isActive ? "bg-emerald-500/15 text-emerald-300" : "bg-white/5 text-slate-500"}`}>
                  {isActive ? "使用中" : "未启用"}
                </span>
                <div className="flex gap-0.5" onClick={(e) => e.stopPropagation()}>
                  <button className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer" title="编辑"
                    onClick={() => { setEditing({ ...s }); setIsNew(false); }}>
                    <Pencil className="w-3 h-3" />
                  </button>
                  <button className="p-1 rounded hover:bg-rose-500/15 text-slate-400 hover:text-rose-300 cursor-pointer" title="删除"
                    onClick={() => remove(s.id)}>
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* 常用分流预设 */}
      <SecondaryPresetPanel running={running} profileId={profileId} />

      {/* 使用说明 */}
      <div className={`${cardCls} p-3 text-[11px] text-slate-400 space-y-1`}>
        <div className="text-slate-300 font-semibold">使用方法</div>
        <div>1. 在「代理」页选中一个节点作为一级代理（例如德国节点），自动作为二级代理的前置。</div>
        <div>2. 在本页点击卡片即可选中/切换二级代理；点「不使用二级代理」则关闭。</div>
        <div>3. 链路：请求 → 一级代理 → 二级代理 → 网站。</div>
        <div>4. 启用后出现「二级代理」策略组：全局模式切到它即全局走二级；规则模式可用规则分流；直连模式不走代理。</div>
      </div>

      {editing && (
        <EditSecondaryModal
          item={editing}
          isNew={isNew}
          busy={saving}
          onCancel={() => { setEditing(null); setIsNew(false); }}
          onSave={(patch) => saveEdit(patch)}
        />
      )}
    </div>
  );
}

function EditSecondaryModal({ item, isNew, busy, onCancel, onSave }: {
  item: SecondaryProxy; isNew: boolean; busy: boolean;
  onCancel: () => void; onSave: (patch: SecondaryProxy) => void;
}) {
  const [form, setForm] = useState<SecondaryProxy>({ ...item });
  return (
    <Modal title={isNew ? "新增二级代理" : "编辑二级代理"} onClose={onCancel} busy={busy} busyText="保存中…"
      footer={
        <>
          <button className={btnSec} disabled={busy} onClick={onCancel}>取消</button>
          <button className={btnPrimary} disabled={busy} onClick={() => onSave(form)}>保存</button>
        </>
      }>
      <div>
        <label className={labelCls}>名称</label>
        <input className={inputCls} placeholder="如：美国1号" value={form.name}
          onChange={(e) => setForm({ ...form, name: e.target.value })} />
      </div>
      <div>
        <label className={labelCls}>地址 (IP/域名)</label>
        <input className={inputCls} placeholder="1.2.3.4 或 home.example.com" value={form.host}
          onChange={(e) => setForm({ ...form, host: e.target.value })} />
      </div>
      <div>
        <label className={labelCls}>端口</label>
        <input className={inputCls} type="number" placeholder="1080" value={form.port || ""}
          onChange={(e) => setForm({ ...form, port: Number(e.target.value) || 0 })} />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls}>账号（可选）</label>
          <input className={inputCls} autoComplete="off" value={form.username || ""}
            onChange={(e) => setForm({ ...form, username: e.target.value })} />
        </div>
        <div>
          <label className={labelCls}>密码（可选）</label>
          <input className={inputCls} type="password" autoComplete="off" value={form.password || ""}
            onChange={(e) => setForm({ ...form, password: e.target.value })} />
        </div>
      </div>
    </Modal>
  );
}

// 常用海外站点分类预设：勾选的具体网站会生成 DOMAIN-SUFFIX 规则走「二级代理」
const SECONDARY_PRESETS: { cat: string; domains: string[] }[] = [
  { cat: "社媒", domains: ["tiktok.com", "facebook.com", "instagram.com", "twitter.com", "x.com", "reddit.com", "pinterest.com", "linkedin.com", "telegram.org", "t.me", "discord.com", "whatsapp.com"] },
  { cat: "视频", domains: ["youtube.com", "youtu.be", "netflix.com", "twitch.tv", "vimeo.com", "dailymotion.com", "hulu.com", "disneyplus.com", "hbomax.com", "paramountplus.com", "spotify.com"] },
  { cat: "电商", domains: ["amazon.com", "ebay.com", "aliexpress.com", "shopify.com", "etsy.com", "walmart.com", "bestbuy.com", "temu.com", "shein.com", "alibaba.com", "paypal.com"] },
  { cat: "游戏", domains: ["steampowered.com", "steamcommunity.com", "epicgames.com", "roblox.com", "ea.com", "ubisoft.com", "nintendo.com", "playstation.com", "xbox.com", "riotgames.com", "battle.net"] },
  { cat: "开发/开源", domains: ["github.com", "gitlab.com", "stackoverflow.com", "npmjs.com", "pypi.org", "docker.com", "huggingface.co", "sourceforge.net", "gitbook.com"] },
  { cat: "AI", domains: ["openai.com", "chatgpt.com", "claude.ai", "anthropic.com", "gemini.google.com", "aistudio.google.com", "groq.com", "mistral.ai", "poe.com"] },
];

function SecondaryPresetPanel({ running, profileId }: { running: boolean; profileId: string }) {
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [applied, setApplied] = useState(false);
  const [err, setErr] = useState("");

  const toggle = (domain: string) => {
    const next = new Set(checked);
    if (next.has(domain)) next.delete(domain);
    else next.add(domain);
    setChecked(next);
    setApplied(false);
  };

  const toggleCat = (cat: string) => {
    const item = SECONDARY_PRESETS.find((c) => c.cat === cat)!;
    const allSelected = item.domains.every((d) => checked.has(d));
    const next = new Set(checked);
    if (allSelected) item.domains.forEach((d) => next.delete(d));
    else item.domains.forEach((d) => next.add(d));
    setChecked(next);
    setApplied(false);
  };

  const toggleCollapse = (cat: string) => {
    const next = new Set(collapsed);
    if (next.has(cat)) next.delete(cat);
    else next.add(cat);
    setCollapsed(next);
  };

  const countByCat = (cat: string) => {
    const item = SECONDARY_PRESETS.find((c) => c.cat === cat)!;
    return item.domains.filter((d) => checked.has(d)).length;
  };

  // 生成规则写入当前订阅的规则覆写 prepend
  const applyRules = async () => {
    if (!checked.size) return;
    if (!profileId) { setErr("未检测到当前订阅，无法写入规则覆写"); return; }
    setBusy(true);
    setErr("");
    try {
      const ov = await mihomoApi.getRuleOverride(profileId);
      const existing = new Set<string>();
      [...(ov?.prepend || []), ...(ov?.append || [])].forEach((r: any) => {
        const s = typeof r === "string" ? r : r?.payload;
        if (s) existing.add(s);
      });
      const newRules = Array.from(checked)
        .map((d) => `DOMAIN-SUFFIX,${d},二级代理`)
        .filter((r) => !existing.has(r));
      if (!newRules.length) {
        setApplied(true);
        setErr("");
        return;
      }
      await mihomoApi.setRuleOverride(profileId, {
        prepend: [...(ov?.prepend || []), ...newRules],
        append: ov?.append || [],
        delete: ov?.delete || [],
      });
      setApplied(true);
    } catch (e: any) {
      setErr(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  // 移除已应用到二级代理的规则（从 prepend 中删除对应 DOMAIN-SUFFIX,xxx,二级代理）
  const removeRules = async () => {
    if (!checked.size) return;
    if (!profileId) { setErr("未检测到当前订阅"); return; }
    setBusy(true);
    setErr("");
    try {
      const ov = await mihomoApi.getRuleOverride(profileId);
      const targets = new Set(Array.from(checked).map((d) => `DOMAIN-SUFFIX,${d},二级代理`));
      const prepend = (ov?.prepend || []).filter((r: any) => {
        const s = typeof r === "string" ? r : r?.payload;
        return !(s && targets.has(s));
      });
      await mihomoApi.setRuleOverride(profileId, {
        prepend,
        append: ov?.append || [],
        delete: ov?.delete || [],
      });
      setApplied(true);
    } catch (e: any) {
      setErr(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const totalChecked = checked.size;

  return (
    <div className={`${cardCls} p-3`}>
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <div className="flex items-center gap-2">
          <Filter className="w-3.5 h-3.5 text-emerald-400" />
          <div>
            <div className="text-[13px] font-bold text-white">常用分流预设</div>
            <div className="text-[10px] text-slate-500">勾选常用海外网站，一键让它们走「二级代理」</div>
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          <button className={btnSec} disabled={busy || !checked.size} onClick={removeRules}>
            <span className="inline-flex items-center gap-1"><X className="w-3 h-3" />移除已选</span>
          </button>
          <button className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-xl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-[12px] font-semibold cursor-pointer disabled:opacity-40"
            disabled={busy || !checked.size} onClick={applyRules}>
            <Check className="w-3.5 h-3.5" /> 添加 {totalChecked ? `(${totalChecked})` : ""} 到二级代理
          </button>
        </div>
      </div>
      {err && <div className="mt-1.5 text-[11px] text-rose-300">{err}</div>}
      {applied && <div className="mt-1.5 text-[11px] text-emerald-300">已应用，请到「规则」页查看或重启内核生效</div>}

      <div className="mt-2.5 grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-2">
        {SECONDARY_PRESETS.map((item) => {
          const isOpen = !collapsed.has(item.cat);
          const allSelected = item.domains.every((d) => checked.has(d));
          const cnt = countByCat(item.cat);
          return (
            <div key={item.cat} className="rounded-lg border border-white/5 bg-white/[0.02] p-2">
              <div className="flex items-center gap-1.5">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={() => toggleCat(item.cat)}
                  className="accent-emerald-500 cursor-pointer"
                />
                <button className="flex-1 text-left text-[12px] font-semibold text-white cursor-pointer flex items-center gap-1"
                  onClick={() => toggleCollapse(item.cat)}>
                  {item.cat}
                  <span className="text-[9px] font-normal text-slate-500">{cnt}/{item.domains.length}</span>
                  <ChevronDown className={`w-3 h-3 text-slate-500 transition-transform ${isOpen ? "rotate-180" : ""}`} />
                </button>
              </div>
              {isOpen && (
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {item.domains.map((d) => (
                    <button key={d}
                      onClick={() => toggle(d)}
                      className={`px-1.5 py-0.5 rounded-md text-[10px] border transition-colors cursor-pointer ${
                        checked.has(d) ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-300" : "bg-white/[0.03] border-white/5 text-slate-400 hover:border-white/20"
                      }`}>
                      {d}
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <div className="mt-2 text-[10px] text-slate-500">
        规则为 DOMAIN-SUFFIX，写入当前订阅的「规则覆写」前置，优先于订阅自带规则。可随时「移除已选」回退。
      </div>
    </div>
  );
}
