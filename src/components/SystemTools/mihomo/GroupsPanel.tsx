// 策略组编辑 + 简易模式（借鉴 clash-party group-editor-modal + simple/* 编译器）
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Plus, Trash2, Save, RotateCcw, ArrowUp, ArrowDown, Wand2 } from "lucide-react";
import { cardCls, btnSec } from "./ui";

type GroupType = "select" | "url-test" | "fallback" | "load-balance" | "relay";
interface Group {
  name: string;
  type: GroupType;
  proxies: string[];
  url?: string;
  interval?: number;
  tolerance?: number;
}
const TYPES: GroupType[] = ["select", "url-test", "fallback", "load-balance", "relay"];
const NEEDS_TEST: GroupType[] = ["url-test", "fallback"];
const DEFAULT_TEST_URL = "http://www.gstatic.com/generate_204";

const inputCls =
  "px-2 py-1 rounded-md bg-slate-900 border border-white/10 text-[11px] text-slate-200 placeholder-slate-600 focus:outline-none focus:border-[var(--module-accent)]/50";

export default function GroupsPanel() {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<Group[]>([]);
  const [nodes, setNodes] = useState<string[]>([]);
  const [hasOverride, setHasOverride] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [simpleGroup, setSimpleGroup] = useState("");

  const load = useCallback(async () => {
    try {
      const d = await invoke<any>("mihomo_get_group_editor");
      setGroups((d?.groups || []).map((g: any) => ({
        name: String(g.name ?? ""),
        type: (g.type ?? "select") as GroupType,
        proxies: Array.isArray(g.proxies) ? g.proxies.map(String) : [],
        url: g.url ? String(g.url) : undefined,
        interval: typeof g.interval === "number" ? g.interval : undefined,
        tolerance: typeof g.tolerance === "number" ? g.tolerance : undefined,
      })));
      setNodes(d?.nodes || []);
      setHasOverride(!!d?.hasOverride);
      setSimpleGroup((v) => v || (d?.groups || [])[0]?.name || "");
    } catch (e: any) {
      setMsg({ ok: false, text: String(e) });
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const patch = (i: number, p: Partial<Group>) =>
    setGroups((gs) => gs.map((g, j) => (j === i ? { ...g, ...p } : g)));

  const move = (i: number, dir: -1 | 1) =>
    setGroups((gs) => {
      const to = i + dir;
      if (to < 0 || to >= gs.length) return gs;
      const next = [...gs];
      [next[i], next[to]] = [next[to], next[i]];
      return next;
    });

  const moveMember = (gi: number, mi: number, dir: -1 | 1) =>
    setGroups((gs) => gs.map((g, j) => {
      if (j !== gi) return g;
      const to = mi + dir;
      if (to < 0 || to >= g.proxies.length) return g;
      const list = [...g.proxies];
      [list[mi], list[to]] = [list[to], list[mi]];
      return { ...g, proxies: list };
    }));

  const addGroup = () =>
    setGroups((gs) => [
      ...gs,
      { name: `${t("groups.newGroup")}${gs.length + 1}`, type: "select", proxies: ["DIRECT"] },
    ]);

  const save = async () => {
    setBusy(true); setMsg(null);
    try {
      await invoke("mihomo_save_proxy_groups", { groups });
      setMsg({ ok: true, text: t("groups.saved") });
      setHasOverride(true);
    } catch (e: any) {
      setMsg({ ok: false, text: String(e) });
    } finally { setBusy(false); }
  };

  const reset = async () => {
    setBusy(true); setMsg(null);
    try {
      await invoke("mihomo_clear_proxy_groups");
      setMsg({ ok: true, text: t("groups.reset") });
      setHasOverride(false);
      await load();
    } catch (e: any) {
      setMsg({ ok: false, text: String(e) });
    } finally { setBusy(false); }
  };

  const applySimple = async (mode: string) => {
    if (!simpleGroup) { setMsg({ ok: false, text: t("groups.pickGroup") }); return; }
    setBusy(true); setMsg(null);
    try {
      await invoke("mihomo_apply_simple_mode", { mode, finalGroup: simpleGroup });
      setMsg({ ok: true, text: t("groups.simpleApplied") });
    } catch (e: any) {
      setMsg({ ok: false, text: String(e) });
    } finally { setBusy(false); }
  };

  return (
    <div className="space-y-3">
      {/* 简易模式：抄 clash-party simple/* 的「可视化编译」，一键生成规则 */}
      <div className={`${cardCls} p-3 space-y-2`}>
        <div className="flex items-center gap-2">
          <Wand2 className="w-3.5 h-3.5 text-[var(--module-accent)]" />
          <span className="text-xs font-semibold text-slate-200">{t("groups.simpleTitle")}</span>
          <div className="flex-1" />
          <select className={inputCls} value={simpleGroup}
            onChange={(e) => setSimpleGroup(e.target.value)}>
            {groups.map((g) => <option key={g.name} value={g.name}>{g.name}</option>)}
          </select>
        </div>
        <div className="text-[10px] text-slate-500">{t("groups.simpleHint")}</div>
        <div className="flex items-center gap-2 flex-wrap">
          {[
            ["rule", "groups.modeRule"],
            ["bypassCN", "groups.modeBypassCN"],
            ["global", "groups.modeGlobal"],
          ].map(([m, label]) => (
            <button key={m} className={btnSec} disabled={busy} onClick={() => void applySimple(m)}>
              {t(label)}
            </button>
          ))}
        </div>
      </div>

      {/* 策略组编辑 */}
      <div className={`${cardCls} p-3 space-y-3`}>
        <div className="flex items-center gap-2">
          <span className="text-xs font-semibold text-slate-200">{t("groups.title")}</span>
          {hasOverride && (
            <span className="px-1.5 py-px rounded bg-[var(--module-accent)]/20 text-[9px] text-[var(--module-accent)]">
              {t("groups.overrideOn")}
            </span>
          )}
          <div className="flex-1" />
          <button className={btnSec} onClick={addGroup}><Plus className="w-3 h-3" />{t("groups.add")}</button>
          <button className={btnSec} onClick={() => void save()} disabled={busy}><Save className="w-3 h-3" />{t("groups.save")}</button>
          <button className={btnSec} onClick={() => void reset()} disabled={busy}><RotateCcw className="w-3 h-3" />{t("groups.restore")}</button>
        </div>
        <div className="text-[10px] text-slate-500">{t("groups.hint")}</div>

        {groups.length === 0 && <div className="text-[11px] text-slate-500">{t("groups.empty")}</div>}

        {groups.map((g, gi) => (
          <div key={gi} className="rounded-lg border border-white/5 bg-slate-900/40 p-2.5 space-y-2">
            <div className="flex items-center gap-2">
              <input className={`${inputCls} flex-1 min-w-0`} value={g.name}
                onChange={(e) => patch(gi, { name: e.target.value })} />
              <select className={inputCls} value={g.type}
                onChange={(e) => patch(gi, { type: e.target.value as GroupType })}>
                {TYPES.map((ty) => <option key={ty} value={ty}>{ty}</option>)}
              </select>
              <button className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer"
                title={t("groups.up")} onClick={() => move(gi, -1)}><ArrowUp className="w-3 h-3" /></button>
              <button className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer"
                title={t("groups.down")} onClick={() => move(gi, 1)}><ArrowDown className="w-3 h-3" /></button>
              <button className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-rose-300 cursor-pointer"
                title={t("groups.del")} onClick={() => setGroups((gs) => gs.filter((_, j) => j !== gi))}>
                <Trash2 className="w-3.5 h-3.5" /></button>
            </div>

            {NEEDS_TEST.includes(g.type) && (
              <div className="flex items-center gap-2 flex-wrap">
                <input className={`${inputCls} flex-1 min-w-[180px]`} value={g.url ?? ""}
                  placeholder={DEFAULT_TEST_URL} onChange={(e) => patch(gi, { url: e.target.value })} />
                <label className="flex items-center gap-1 text-[10px] text-slate-400">
                  {t("groups.interval")}
                  <input className={`${inputCls} w-20`} type="number" min={10}
                    value={g.interval ?? 300} onChange={(e) => patch(gi, { interval: Number(e.target.value) || 300 })} />
                </label>
                <label className="flex items-center gap-1 text-[10px] text-slate-400">
                  {t("groups.tolerance")}
                  <input className={`${inputCls} w-16`} type="number" min={0}
                    value={g.tolerance ?? 0} onChange={(e) => patch(gi, { tolerance: Number(e.target.value) || 0 })} />
                </label>
              </div>
            )}

            <div className="space-y-1">
              {g.proxies.map((p, mi) => (
                <div key={`${p}-${mi}`} className="flex items-center gap-1.5">
                  <span className="flex-1 min-w-0 truncate text-[11px] text-slate-300">{p}</span>
                  <button className="p-0.5 rounded hover:bg-white/10 text-slate-500 cursor-pointer"
                    onClick={() => moveMember(gi, mi, -1)}><ArrowUp className="w-3 h-3" /></button>
                  <button className="p-0.5 rounded hover:bg-white/10 text-slate-500 cursor-pointer"
                    onClick={() => moveMember(gi, mi, 1)}><ArrowDown className="w-3 h-3" /></button>
                  <button className="p-0.5 rounded hover:bg-white/10 text-slate-500 hover:text-rose-300 cursor-pointer"
                    onClick={() => setGroups((gs) => gs.map((x, j) => j === gi ? { ...x, proxies: x.proxies.filter((_, k) => k !== mi) } : x))}>
                    <Trash2 className="w-3 h-3" /></button>
                </div>
              ))}
              <select className={inputCls} value=""
                onChange={(e) => {
                  const v = e.target.value;
                  if (!v) return;
                  setGroups((gs) => gs.map((x, j) => j === gi ? { ...x, proxies: [...x.proxies, v] } : x));
                }}>
                <option value="">{t("groups.addMember")}</option>
                {nodes.map((n) => <option key={n} value={n}>{n}</option>)}
              </select>
            </div>
          </div>
        ))}
      </div>

      {msg && <div className={`text-[11px] ${msg.ok ? "text-emerald-400" : "text-rose-300"}`}>{msg.text}</div>}
    </div>
  );
}
