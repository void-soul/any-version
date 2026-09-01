// 代理页 —— 1:1 复刻 clash-party src/renderer/src/pages/proxies.tsx 行为
import { useEffect, useRef, useState } from "react";
import { Zap, ChevronDown, ChevronUp, Search, Pin, LocateFixed, ArrowDownUp, RefreshCw, X } from "lucide-react";
import "flag-icons/css/flag-icons.min.css";
import { mihomoApi } from "../mihomoApi";
import {
  IMihomoMixedGroup, IMihomoProxy, getMixedGroups, lastDelay, changeProxy, unfixedProxy,
  proxyDelay, groupDelay, pooledDelayTest, ctrlGet, closeConnection,
  FAKE_GROUP_TYPE,
} from "./ctrl";
import { btnSec, cardCls, delayColor, delayText, Toggle } from "./ui";
import { useTranslation } from "react-i18next";
import { nodeFlag } from "./flag";

/** 节点国旗；识别不到时留出等宽占位，保证网格对齐 */
function Flag({ name }: { name: string }) {
  const code = nodeFlag(name);
  if (!code) return <span className="inline-block w-4 shrink-0" />;
  return <span className={`fi fi-${code} shrink-0 rounded-[2px]`} style={{ width: 16, height: 12 }} />;
}

const DEFAULT_DELAY_URL = "https://www.gstatic.com/generate_204";

type OrderMode = "default" | "delay" | "name";
const ORDER_LABEL: Record<OrderMode, string> = { default: "defaultOrder", delay: "delayOrder", name: "nameOrder" };
const NEXT_ORDER: Record<OrderMode, OrderMode> = { default: "delay", delay: "name", name: "default" };

export default function ProxiesPanel({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<IMihomoMixedGroup[]>([]);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [search, setSearch] = useState<Record<string, string>>({});
  const [testing, setTesting] = useState<Record<string, Set<string>>>({});
  // 某组某节点是否正在测速（对齐 clash-party 的 delaying[index]?.has(name)）
  const isTesting = (g: IMihomoMixedGroup, name: string) => !!testing[g.name]?.has(name);
  const anyTesting = () => Object.values(testing).some((s) => s.size > 0);
  const [cfg, setCfg] = useState<any>(null);
  const [profiles, setProfiles] = useState<any[]>([]);
  const [curProfile, setCurProfile] = useState<string>("");
  const [err, setErr] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  // 刚刚手动切换过的组，短时间内不让轮询结果覆盖乐观值
  const pinnedRef = useRef<Record<string, number>>({});
  // 切换后延时刷新定时器句柄，组件卸载时清理，避免对已卸载组件 setState
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const order: OrderMode = (cfg?.proxyDisplayOrder as OrderMode) || "default";
  const mode: "full" | "simple" = cfg?.proxyDisplayMode === "simple" ? "simple" : "full";
  const cols: number = Number(cfg?.proxy_cols) || 2;
  const delayUrl: string = cfg?.delayTestUrl || DEFAULT_DELAY_URL;
  const delayTimeout: number = Number(cfg?.delayTestTimeout) || 5000;
  const concurrency: number = Number(cfg?.delayTestConcurrency) || 50;
  const autoClose: boolean = !!cfg?.autoCloseConnection;

  const refresh = async () => {
    try {
      const next = await getMixedGroups();
      const now = Date.now();
      setGroups((prev) =>
        next.map((g) => {
          // 手动切换后的 1.5s 内保留本地选中值，避免核心尚未同步导致选中态闪回
          if ((pinnedRef.current[g.name] || 0) > now) {
            const old = prev.find((x) => x.name === g.name);
            if (old) return { ...g, now: old.now };
          }
          return g;
        })
      );
    } catch (e: any) {
      setGroups([]);
    }
  };

  const manualRefresh = async () => {
    setRefreshing(true);
    try {
      pinnedRef.current = {};
      setGroups(await getMixedGroups());
      setErr("");
    } catch (e: any) {
      setErr(t("proxies.refreshFail", { err: String(e) }));
    } finally {
      setRefreshing(false);
    }
  };
  const loadCfg = async () => { try { setCfg(await mihomoApi.getAppConfig()); } catch {} };
  const patchCfg = async (p: any) => { await mihomoApi.patchAppConfig(p); loadCfg(); };

  useEffect(() => {
    loadCfg();
    if (running) {
      mihomoApi.getProfileConfig()
        .then((c: any) => { setProfiles(c?.items || []); setCurProfile(c?.current || ""); })
        .catch(() => {});
      // 切换订阅后核心经 PATCH /configs 异步重载，需稍候再拉取；挂载时做「即时 + 两次延迟」刷新
      refresh();
      const t1 = setTimeout(refresh, 600);
      const t2 = setTimeout(refresh, 1500);
      const t = setInterval(refresh, 5000); // 对齐 clash-party SWR 轮询
      return () => { clearTimeout(t1); clearTimeout(t2); clearInterval(t); };
    }
    setProfiles([]); setCurProfile(""); setGroups([]);
  }, [running]);

  // 组件卸载时清理切换后的延时刷新定时器
  useEffect(() => () => {
    if (refreshTimer.current) clearTimeout(refreshTimer.current);
  }, []);

  // 订阅列表中切换当前订阅后立即刷新（不等待轮询），并同步当前配置标签
  useEffect(() => {
    const onChanged = () => {
      mihomoApi
        .getProfileConfig()
        .then((c: any) => { setProfiles(c?.items || []); setCurProfile(c?.current || ""); })
        .catch(() => {});
      refresh();
      setTimeout(refresh, 600);
      setTimeout(refresh, 1500);
    };
    window.addEventListener("mihomo:profile-changed", onChanged);
    return () => window.removeEventListener("mihomo:profile-changed", onChanged);
  }, []);

  // 排序（复刻 clash-party：delay 时未测/超时排最后；name 用 localeCompare）
  const sortProxies = (all: IMihomoProxy[]): IMihomoProxy[] => {
    if (order === "default") return all;
    const arr = [...all];
    if (order === "delay") {
      arr.sort((a, b) => {
        const da = lastDelay(a); const db = lastDelay(b);
        if (da === -1 && db === -1) return 0;
        if (da === -1) return 1;
        if (db === -1) return -1;
        if (da === 0 && db === 0) return 0;
        if (da === 0) return 1;
        if (db === 0) return -1;
        return da - db;
      });
    } else {
      arr.sort((a, b) => a.name.localeCompare(b.name));
    }
    return arr;
  };

  // 选择节点（复刻 onChangeProxy：URLTest/Fallback 是"固定"，可选断开旧连接）
  const onChangeProxy = async (group: IMihomoMixedGroup, proxy: string) => {
    // 「全部节点」假组在内核中不存在，无法切换，跳过避免 404
    if (group.type === FAKE_GROUP_TYPE) return;
    if (group.now === proxy) return;
    setErr("");
    // 乐观更新：先切 UI，失败再回滚并提示（此前失败被静默吞掉，表现为"点了没反应"）
    const prevNow = group.now;
    pinnedRef.current[group.name] = Date.now() + 1500;
    setGroups((gs) => gs.map((x) => (x.name === group.name ? { ...x, now: proxy } : x)));
    try {
      await changeProxy(group.name, proxy);
    } catch (e: any) {
      delete pinnedRef.current[group.name];
      setGroups((gs) => gs.map((x) => (x.name === group.name ? { ...x, now: prevNow } : x)));
      setErr(t("proxies.switchFail", { proxy, err: String(e).replace(/^Error:\s*/, "") }));
      return;
    }
    // 将选中的节点持久化为「一级代理」（供二级代理作为 dialer-proxy 前置）
    try { await mihomoApi.selectProxy(proxy); } catch {}
    if (autoClose) {
      // 复刻 clash-party：断开经由该组的旧连接
      try {
        const res = await ctrlGet("/connections");
        for (const c of res?.connections || []) {
          if ((c.chains || []).includes(group.name)) await closeConnection(c.id);
        }
      } catch {}
    }
    refreshTimer.current = setTimeout(refresh, 300);
  };

  const onUnfix = async (group: IMihomoMixedGroup) => {
    try {
      await unfixedProxy(group.name);
    } catch (e: any) {
      setErr(t("proxies.unpinFail", { err: String(e) }));
    }
    refresh();
  };

  // 计算某组的测速目标节点名（有搜索则仅命中项）
  const delayTargets = (g: IMihomoMixedGroup): string[] => {
    const kw = (search[g.name] || "").trim().toLowerCase();
    return (kw ? g.all.filter((p) => p.name.toLowerCase().includes(kw)) : g.all).map((p) => p.name);
  };
  // 实际发起测速（无搜索走核心组测速；有搜索走前端并发池）
  const runDelay = (g: IMihomoMixedGroup, names: string[]): Promise<void> => {
    const url = g.testUrl || delayUrl;
    if (!names.length) return Promise.resolve();
    return (search[g.name]?.trim()
      ? pooledDelayTest(names, url, delayTimeout, concurrency)
      : groupDelay(g.name, url, delayTimeout));
  };

  // 组延迟测试（复刻 clash-party：整组节点标记为测速中，结束后刷新）
  const onGroupDelay = async (group: IMihomoMixedGroup) => {
    if (testing[group.name]?.size) return; // 已在测速，避免重复触发
    const names = delayTargets(group);
    if (!names.length) return;
    setTesting((s) => ({ ...s, [group.name]: new Set(names) }));
    try {
      await runDelay(group, names);
    } catch (e: any) {
      setErr(t("proxies.testGroupFail", { err: String(e) }));
    } finally {
      setTesting((s) => {
        const n = { ...s };
        delete n[group.name];
        return n;
      });
      refresh();
    }
  };

  /** 全部组并发测速（所有组节点同时显示加载效果） */
  const onTestAll = async () => {
    if (anyTesting()) return;
    setTesting(Object.fromEntries(groups.map((g) => [g.name, new Set(delayTargets(g))] as const)));
    try {
      await Promise.all(
        groups.map((g) =>
          runDelay(g, delayTargets(g)).catch((e: any) => setErr(t("proxies.testAllFail", { err: String(e) })))
        )
      );
    } finally {
      setTesting({});
      refresh();
    }
  };

  const onProxyDelay = async (group: IMihomoMixedGroup, name: string) => {
    try {
      await proxyDelay(name, group.testUrl || delayUrl, delayTimeout);
    } catch (e: any) {
      setErr(t("proxies.testNodeFail", { name, err: String(e) }));
    }
    refresh();
  };

  // 定位当前节点（复刻 LocateFixed 按钮）
  const locate = (group: IMihomoMixedGroup) => {
    setOpen((s) => ({ ...s, [group.name]: true }));
    setTimeout(() => {
      const el = document.getElementById(`mihomo-proxy-${group.name}-${group.now}`);
      el?.scrollIntoView({ behavior: "smooth", block: "center" });
    }, 100);
  };

  if (!running) {
    return <div className={`${cardCls} p-6 text-center text-xs text-slate-400`}>{t("proxies.coreNotRunning")}</div>;
  }

  return (
    <div className="space-y-3" ref={listRef}>
      {/* 顶栏：排序 / 显示模式 / 列数（对齐 clash-party 顶栏控制）*/}
      <div className="flex items-center gap-2 flex-wrap">
        <button className={btnSec} onClick={() => patchCfg({ proxyDisplayOrder: NEXT_ORDER[order] })}>
          <span className="inline-flex items-center gap-1"><ArrowDownUp className="w-3 h-3" />{t(`proxies.${ORDER_LABEL[order]}`)}</span>
        </button>
        <button className={btnSec} onClick={() => patchCfg({ proxyDisplayMode: mode === "full" ? "simple" : "full" })}>
          {mode === "full" ? t("proxies.detailMode") : t("proxies.simpleMode")}
        </button>
        {/* 列数切换：紧凑分段按钮，避免文字过大被截断 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] text-slate-400">{t("proxies.columns")}</span>
          <div className="flex items-center rounded-lg bg-white/10 p-0.5">
            {[1, 2, 3, 4].map((c) => (
              <button
                key={c}
                onClick={() => patchCfg({ proxy_cols: c })}
                className={`min-w-[22px] h-6 rounded-md text-[11px] font-semibold transition-colors cursor-pointer ${
                  cols === c ? "bg-white/20 text-white" : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {c}
              </button>
            ))}
          </div>
        </div>
        <button className={btnSec} onClick={manualRefresh} disabled={refreshing}>
          <span className="inline-flex items-center gap-1">
            <RefreshCw className={`w-3 h-3 ${refreshing ? "animate-spin" : ""}`} />
            {t("proxies.refresh")}
          </span>
        </button>
        <button className={btnSec} onClick={onTestAll} disabled={anyTesting() || groups.length === 0}>
          <span className="inline-flex items-center gap-1">
            <Zap className={`w-3 h-3 ${anyTesting() ? "animate-spin" : ""}`} />
            {t("proxies.testAll")}
          </span>
        </button>
        <div className="flex-1" />
        <Toggle label={t("proxies.autoClose")} v={autoClose} onChange={(v) => patchCfg({ autoCloseConnection: v })} />
      </div>

      {err && (
        <div className="flex items-start gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-200">
          <span className="flex-1 break-all">{err}</span>
          <button className="text-rose-300 hover:text-white cursor-pointer" onClick={() => setErr("")}>
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {groups.length === 0 && (
        <div className={`${cardCls} p-6 text-center text-sm text-slate-300 space-y-3`}>
          <div className="text-base font-semibold text-white">{t("proxies.noNodes")}</div>
          <div className="text-xs text-slate-400">
            {t("proxies.noNodesDesc1", { profile: curProfile || t("common.none") })}
            {t("proxies.noNodesDesc2")}
          </div>
          {profiles.filter((p) => p.id !== curProfile).length > 0 && (
            <div className="flex flex-wrap items-center justify-center gap-2">
              <span className="text-[11px] text-slate-500">{t("proxies.quickSwitch")}</span>
              {profiles.filter((p) => p.id !== curProfile).map((p) => (
                <button
                  key={p.id}
                  className="px-2.5 py-1 rounded-lg bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] text-[11px] text-[var(--module-accent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] cursor-pointer"
                  onClick={async () => {
                    try {
                      await mihomoApi.changeCurrentProfile(p.id);
                      setTimeout(() => refresh(), 1200);
                    } catch {}
                  }}
                >
                  {p.name}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      {groups.map((g) => {
        const expanded = !!open[g.name];
        const kw = (search[g.name] || "").trim().toLowerCase();
        const shown = sortProxies(kw ? g.all.filter((p) => p.name.toLowerCase().includes(kw)) : g.all);
        return (
          <div key={g.name} className={`${cardCls} overflow-hidden`}>
            {/* 组头 */}
            <div
              className="flex items-center gap-3 px-4 py-3 cursor-pointer hover:bg-white/[0.03]"
              onClick={() => setOpen((s) => ({ ...s, [g.name]: !expanded }))}
            >
              {g.icon ? (
                <img src={g.icon} className="w-8 h-8 rounded-lg object-contain bg-white/5" onError={(e) => ((e.target as HTMLImageElement).style.display = "none")} />
              ) : null}
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-[13px] font-semibold text-white truncate">{g.name}</span>
                </div>
                <div className="flex items-center gap-1.5 text-[11px] text-slate-400 truncate">
                  <span>{g.type} ·</span>
                  <Flag name={g.now} />
                  <span className="truncate">{g.now}</span>
                </div>
              </div>
              <span className="text-[11px] text-slate-500">{g.all.length}</span>
              <button
                className="p-1.5 rounded-lg hover:bg-white/10 text-slate-300 cursor-pointer"
                title={t("proxies.locateNode")}
                onClick={(e) => { e.stopPropagation(); locate(g); }}
              >
                <LocateFixed className="w-3.5 h-3.5" />
              </button>
              <button
                className={`p-1.5 rounded-lg hover:bg-white/10 cursor-pointer ${g.fixed ? "text-amber-300" : "text-slate-500"}`}
                title={g.fixed ? t("proxies.unpin") : t("proxies.notPinned")}
                onClick={(e) => { e.stopPropagation(); onUnfix(g); }}
              >
                <Pin className={`w-3.5 h-3.5 ${g.fixed ? "fill-current" : ""}`} />
              </button>
              <button
                className="p-1.5 rounded-lg hover:bg-white/10 text-[var(--module-accent)] cursor-pointer disabled:opacity-50"
                title={t("proxies.delayTest")}
                disabled={!!testing[g.name]?.size}
                onClick={(e) => { e.stopPropagation(); onGroupDelay(g); }}
              >
                {testing[g.name]?.size ? (
                  <span className="block w-3.5 h-3.5 rounded-full border-2 border-[var(--module-accent-ring)] border-t-[var(--module-accent)] animate-spin" />
                ) : (
                  <Zap className="w-3.5 h-3.5" />
                )}
              </button>
              {expanded ? <ChevronUp className="w-4 h-4 text-slate-400" /> : <ChevronDown className="w-4 h-4 text-slate-400" />}
            </div>

            {/* 展开区 */}
            {expanded && (
              <div className="border-t border-white/5 p-3 space-y-2">
                <div className="relative">
                  <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-500" />
                  <input
                    className="w-full h-8 pl-8 pr-2.5 rounded-lg bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
                    placeholder={t("proxies.searchNode")}
                    value={search[g.name] || ""}
                    onChange={(e) => setSearch((s) => ({ ...s, [g.name]: e.target.value }))}
                  />
                </div>
                <div className={`grid gap-1.5`} style={{ gridTemplateColumns: `repeat(${cols}, minmax(0,1fr))` }}>
                  {shown.map((p) => {
                    const selected = g.now === p.name;
                    const d = lastDelay(p);
                    return (
                      <div
                        key={p.name}
                        id={`mihomo-proxy-${g.name}-${p.name}`}
                        onClick={() => onChangeProxy(g, p.name)}
                        className={`px-2.5 py-2 rounded-xl border cursor-pointer transition-all ${
                          selected
                            ? "bg-[var(--module-accent-soft)] border-[var(--module-accent-ring)]"
                            : "bg-white/[0.03] border-white/5 hover:border-white/20"
                        }`}
                      >
                        <div className="flex items-center gap-1.5">
                          <Flag name={p.name} />
                          <span className={`text-[12px] truncate flex-1 ${selected ? "text-[var(--module-accent)] font-semibold" : "text-slate-200"}`}>
                            {p.name}
                          </span>
                          <button
                            className={`text-[10px] font-mono flex-shrink-0 cursor-pointer hover:underline ${delayColor(d)}`}
                            title={t("proxies.testDelay")}
                            onClick={(e) => { e.stopPropagation(); onProxyDelay(g, p.name); }}
                          >
                            {isTesting(g, p.name) ? (
                              <span className="inline-block w-3 h-3 rounded-full border border-white/30 border-t-white animate-spin align-middle" />
                            ) : (
                              t(delayText(d))
                            )}
                          </button>
                        </div>
                        {mode === "full" && (
                          <div className="mt-0.5 text-[10px] text-slate-500 truncate">
                            {p.type}
                            {p.udp ? " · UDP" : ""}
                            {p.provider ? ` · ${p.provider}` : ""}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
