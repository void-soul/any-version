// 内核设置页 —— 1:1 复刻 clash-party src/renderer/src/pages/mihomo.tsx（win32 分支）
// （混合/Socks/HTTP 端口：数字+随机+启停+确认；外部控制器+校验；密钥生成/显隐；
//   WebUI 面板选择/更新/打开；IPv6；允许局域网+允许/禁止网段；用户验证；
//   跳过验证前缀；RTT 延迟；TCP 并发；记住节点/FakeIP；日志等级；进程查找；核心升级）
import { useEffect, useState } from "react";
import { Shuffle, RefreshCw, Eye, EyeOff, Trash2, ExternalLink, CloudDownload } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, SettingItem, Toggle, btnSec, btnPrimary, inputCls } from "./ui";

// 复刻 shared/appConfig 默认值
const DEFAULT_PORTS = { mixed: 7890, socks: 7891, http: 7892 };
const DEFAULT_SKIP_AUTH = ["127.0.0.1/32", "::1/128"];
const DEFAULT_LAN_ALLOWED = ["0.0.0.0/0", "::/0"];

const WEBUI_PANELS = [
  { name: "zashboard", url: "https://github.com/Zephyruso/zashboard/releases/latest/download/dist.zip" },
  { name: "metacubexd", url: "https://github.com/MetaCubeX/metacubexd/archive/refs/heads/gh-pages.zip" },
  { name: "yacd-meta", url: "https://github.com/MetaCubeX/Yacd-meta/archive/refs/heads/gh-pages.zip" },
  { name: "yacd", url: "https://github.com/haishanh/yacd/archive/refs/heads/gh-pages.zip" },
  { name: "razord-meta", url: "https://github.com/MetaCubeX/Razord-meta/archive/refs/heads/gh-pages.zip" },
];

// 复刻 isValidListenAddress（host:port 或 :port）
function isValidListenAddress(s: string): boolean {
  if (!s) return false;
  const m = s.match(/^(.*):(\d{1,5})$/);
  if (!m) return false;
  const port = parseInt(m[2]);
  return port > 0 && port <= 65535;
}

const randomPort = () => Math.floor(Math.random() * (65535 - 1024 + 1)) + 1024;
const randomSecret = () => {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  return Array.from({ length: 8 }, () => chars[Math.floor(Math.random() * chars.length)]).join("");
};

export default function CorePanel({ onCoreChanged }: { onCoreChanged?: () => void }) {
  const [app, setApp] = useState<any>({});
  const [c, setC] = useState<any>({});
  const [msg, setMsg] = useState("");
  const [upgrading, setUpgrading] = useState(false);
  const [upgradingUi, setUpgradingUi] = useState(false);
  // 端口输入态（复刻 showXxxPort 输入草稿模型）
  const [portInputs, setPortInputs] = useState<Record<string, number>>({});
  const [ecInput, setEcInput] = useState("");
  const [secretInput, setSecretInput] = useState("");
  const [secretVisible, setSecretVisible] = useState(false);
  const [lanAllowed, setLanAllowed] = useState<string[]>([]);
  const [lanDisallowed, setLanDisallowed] = useState<string[]>([]);
  const [auth, setAuth] = useState<string[]>([]);
  const [skipAuth, setSkipAuth] = useState<string[]>(DEFAULT_SKIP_AUTH);
  const [variants, setVariants] = useState<any[]>([]);

  const loadVariants = () =>
    mihomoApi
      .coreVariants()
      .then((v: any) => setVariants(Array.isArray(v?.items) ? v.items : []))
      .catch(() => {});

  const load = async () => {
    try {
      const [a, cc]: any[] = await Promise.all([mihomoApi.getAppConfig(), mihomoApi.getControledConfig()]);
      setApp(a || {});
      setC(cc || {});
      loadVariants();
      setPortInputs({
        mixed: cc?.["mixed-port"] ?? DEFAULT_PORTS.mixed,
        socks: cc?.["socks-port"] ?? DEFAULT_PORTS.socks,
        http: cc?.port ?? DEFAULT_PORTS.http,
      });
      setEcInput(cc?.["external-controller"] ?? "");
      setSecretInput(cc?.secret ?? "");
      setLanAllowed(cc?.["lan-allowed-ips"] ?? DEFAULT_LAN_ALLOWED);
      setLanDisallowed(cc?.["lan-disallowed-ips"] ?? []);
      setAuth(cc?.authentication ?? []);
      setSkipAuth(cc?.["skip-auth-prefixes"] ?? DEFAULT_SKIP_AUTH);
    } catch (err) {
      console.error("加载核心配置失败:", err);
    }
  };
  useEffect(() => { load(); }, []);

  // 复刻 onChangeNeedRestart：patch controled → 重启核心生效
  const onChangeNeedRestart = async (patch: any) => {
    try {
      await mihomoApi.patchControledConfig(patch);
      await mihomoApi.restart();
      setMsg("已应用并重启核心");
      onCoreChanged?.();
    } catch (e: any) {
      setMsg(`应用失败: ${e}`);
    }
    load();
  };

  // 修改应用侧配置（need_restart 为 true 时重启内核使之生效）
  const patchApp = async (patch: any, needRestart = false) => {
    try {
      await mihomoApi.patchAppConfig(patch);
      if (needRestart) {
        await mihomoApi.restart();
        setMsg("已应用并重启核心");
        onCoreChanged?.();
      } else {
        setMsg("已保存");
      }
    } catch (e: any) {
      setMsg(`保存失败: ${e}`);
    }
    load();
  };

  const ecValid = isValidListenAddress(ecInput);

  // 复刻 openExternalUi 的 URL 组装
  const openExternalUi = () => {
    try {
      let controller = (c?.["external-controller"] || "").trim();
      if (controller.startsWith(":")) controller = `127.0.0.1${controller}`;
      if (controller.startsWith("0.0.0.0:")) controller = controller.replace("0.0.0.0:", "127.0.0.1:");
      const u = new URL(`http://${controller}`);
      const params = new URLSearchParams({ hostname: u.hostname, port: u.port });
      const secret = c?.secret || "";
      if (secret) params.set("secret", secret);
      const panel = (c?.["external-ui-url"] || WEBUI_PANELS[0].url).toLowerCase();
      let url = `${u.origin}/ui/?${params.toString()}`;
      if (panel.includes("zashboard") || panel.includes("metacubexd")) {
        url = `${u.origin}/ui/#/setup?${params.toString()}`;
      } else if (panel.includes("razord")) {
        params.set("host", u.hostname);
        params.delete("hostname");
        url = `${u.origin}/ui/#/proxies?${params.toString()}`;
      }
      window.open(url, "_blank", "noopener,noreferrer");
    } catch {
      setMsg("外部控制器地址无效");
    }
  };

  const upgradeUi = async () => {
    setUpgradingUi(true);
    try { await mihomoApi.upgradeUi(); setMsg("WebUI 已更新"); }
    catch (e: any) { setMsg(`WebUI 更新失败: ${e}`); }
    finally { setUpgradingUi(false); }
  };

  // 端口行（复刻 mixed/socks/http 三行：确认+随机+启停）
  const renderPortRow = (
    title: string, key: "mixed" | "socks" | "http",
    cKey: string, enableKey: string, defaultPort: number
  ) => {
    const current = c?.[cKey] ?? defaultPort;
    const input = portInputs[key] ?? current;
    const enabled = app?.[enableKey] ?? true;
    return (
      <SettingItem title={title}>
        <div className="flex items-center gap-2">
          {input !== current && enabled && (
            <button className={btnPrimary} onClick={() => onChangeNeedRestart({ [cKey]: input })}>确认</button>
          )}
          <input
            className={`${inputCls} !w-24`} type="number" min={0} max={65535}
            value={input}
            disabled={!enabled}
            onChange={(e) => {
              const p = e.target.value === "" ? 0 : parseInt(e.target.value);
              if (!isNaN(p) && p >= 0 && p <= 65535) setPortInputs((s) => ({ ...s, [key]: p }));
            }}
          />
          <button className={btnSec} title="随机端口" onClick={() => setPortInputs((s) => ({ ...s, [key]: randomPort() }))}>
            <Shuffle className="w-3.5 h-3.5" />
          </button>
          <Toggle v={enabled} onChange={async (v) => {
            await mihomoApi.patchAppConfig({ [enableKey]: v });
            await onChangeNeedRestart({ [cKey]: v ? input || defaultPort : 0 });
          }} />
        </div>
      </SettingItem>
    );
  };

  // 通用字符串列表编辑（复刻 [...list, ''] 输入模型）
  const renderList = (
    list: string[], setList: (l: string[]) => void, placeholder: string,
    saved: string[], onConfirm: () => void, lockFirstTwo = false
  ) => (
    <>
      {JSON.stringify(list) !== JSON.stringify(saved) && (
        <div className="flex justify-end mb-1"><button className={btnPrimary} onClick={onConfirm}>确认</button></div>
      )}
      {[...list, ""].map((item, index) => (
        <div key={index} className="flex mb-1.5 gap-2">
          <input
            className={inputCls} placeholder={placeholder} value={item || ""}
            disabled={lockFirstTwo && (index === 0 || index === 1)}
            onChange={(e) => {
              const v = e.target.value;
              if (index === list.length) setList([...list, v]);
              else setList(list.map((a, i) => (i === index ? v : a)));
            }}
          />
          {index < list.length && !(lockFirstTwo && (index === 0 || index === 1)) && (
            <button className={btnSec} onClick={() => setList(list.filter((_, i) => i !== index))}>
              <Trash2 className="w-3.5 h-3.5 text-amber-300" />
            </button>
          )}
        </div>
      ))}
    </>
  );

  const externalUiEnabled = (c?.["external-ui"] || "") === "ui";
  const externalUiUrl = c?.["external-ui-url"] || WEBUI_PANELS[0].url;

  const currentVariant = app?.core || "mihomo";
  const isSmartCore = currentVariant === "mihomo-smart";

  return (
    <div className="space-y-3">
      {/* 内核版本切换：三个内核随程序预置于 bin/mihomo，无需联网安装 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">内核版本</h3>
          <span className="text-[10px] text-slate-500">内核已随程序预置，直接切换即可</span>
        </div>
        {[
          ["mihomo", "稳定版", "MetaCubeX/mihomo Release"],
          ["mihomo-alpha", "预览版", "MetaCubeX/mihomo Alpha"],
          ["mihomo-smart", "Smart 版", "vernesong/mihomo 智能内核"],
        ].map(([id, name, desc], idx, arr) => {
          const info = variants.find((v) => v.id === id);
          const active = currentVariant === id;
          return (
            <SettingItem
              key={id}
              divider={idx !== arr.length - 1}
              title={
                <span className="inline-flex items-center gap-2">
                  {name}
                  {active && <span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)]">使用中</span>}
                  <span className="text-[10px] text-slate-500">{info?.version || desc}</span>
                </span>
              }
            >
              <div className="flex items-center gap-2">
                <span className={`text-[10px] ${info?.installed ? "text-slate-500" : "text-rose-400"}`}>
                  {info?.installed ? "已就绪" : "文件缺失"}
                </span>
                <button
                  className={btnPrimary}
                  disabled={active || !info?.installed}
                  title={info?.installed ? "" : `内核文件不存在：${info?.path || ""}`}
                  onClick={() => patchApp({ core: id }, true)}
                >
                  切换
                </button>
              </div>
            </SettingItem>
          );
        })}
      </div>

      {/* Smart 内核参数（对齐 clash-party enableSmartOverride 系列设置） */}
      {isSmartCore && (
        <div className={`${cardCls} p-4`}>
          <h3 className="text-sm font-bold text-white mb-1">Smart 内核</h3>
          <SettingItem title={<span title="自动把 url-test / load-balance 组转换为 smart 组">启用 Smart 覆写</span>}>
            <Toggle v={app?.enableSmartOverride !== false} onChange={(v) => patchApp({ enableSmartOverride: v }, true)} />
          </SettingItem>
          <SettingItem title="策略">
            <select
              className="h-8 px-2 rounded-lg bg-white/10 border border-white/10 text-[11px] text-slate-200 cursor-pointer focus:outline-none"
              value={app?.smartCoreStrategy ?? "sticky-sessions"}
              onChange={(e) => patchApp({ smartCoreStrategy: e.target.value }, true)}
            >
              {["sticky-sessions", "round-robin"].map((s) => (
                <option key={s} value={s} className="bg-slate-800">{s}</option>
              ))}
            </select>
          </SettingItem>
          <SettingItem title={<span title="使用 LightGBM 模型进行决策">使用 LightGBM</span>}>
            <Toggle v={!!app?.smartCoreUseLightGBM} onChange={(v) => patchApp({ smartCoreUseLightGBM: v }, true)} />
          </SettingItem>
          <SettingItem title={<span title="收集连接数据用于模型训练">收集训练数据</span>}>
            <Toggle v={!!app?.smartCoreCollectData} onChange={(v) => patchApp({ smartCoreCollectData: v }, true)} />
          </SettingItem>
          <SettingItem title="采集器大小 (MB)" divider={false}>
            <input
              className={`${inputCls} !w-24`}
              type="number"
              defaultValue={app?.smartCollectorSize ?? 100}
              onBlur={(e) => {
                const n = parseInt(e.target.value);
                if (!isNaN(n) && n !== (app?.smartCollectorSize ?? 100)) patchApp({ smartCollectorSize: n }, true);
              }}
            />
          </SettingItem>
        </div>
      )}

      <div className={`${cardCls} p-4`}>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-bold text-white">内核设置</h3>
          <div className="flex items-center gap-2">
            {msg && <span className="text-[11px] text-slate-400">{msg}</span>}
            <button
              className={btnSec}
              disabled={upgrading}
              onClick={async () => {
                setUpgrading(true);
                try { await mihomoApi.upgradeCore(); setMsg("核心升级完成"); onCoreChanged?.(); }
                catch (e: any) { setMsg(String(e).includes("already") ? "已是最新版本" : `升级失败: ${e}`); }
                finally { setUpgrading(false); }
              }}
            >
              <span className="inline-flex items-center gap-1">
                <CloudDownload className={`w-3.5 h-3.5 ${upgrading ? "animate-bounce" : ""}`} />升级核心
              </span>
            </button>
          </div>
        </div>

        {renderPortRow("混合端口", "mixed", "mixed-port", "enableMixedPort", DEFAULT_PORTS.mixed)}
        {renderPortRow("Socks 端口", "socks", "socks-port", "enableSocksPort", DEFAULT_PORTS.socks)}
        {renderPortRow("HTTP 端口", "http", "port", "enableHttpPort", DEFAULT_PORTS.http)}

        <SettingItem title="外部控制器">
          <div className="flex items-center gap-2">
            {ecInput !== (c?.["external-controller"] || "") && ecValid && (
              <button className={btnPrimary} onClick={() => onChangeNeedRestart({ "external-controller": ecInput })}>确认</button>
            )}
            <input
              className={`${inputCls} !w-52 ${!ecValid && ecInput ? "!border-rose-500" : ""}`}
              placeholder="127.0.0.1:9090"
              value={ecInput}
              onChange={(e) => setEcInput(e.target.value)}
            />
          </div>
        </SettingItem>

        <SettingItem title={<span className="inline-flex items-center gap-1">外部控制器密钥
          <button className="p-0.5 rounded hover:bg-white/10 cursor-pointer" title="随机生成" onClick={() => setSecretInput(randomSecret())}>
            <RefreshCw className="w-3 h-3 text-slate-400" />
          </button></span>}>
          <div className="flex items-center gap-2">
            {secretInput !== (c?.secret || "") && (
              <button className={btnPrimary} onClick={() => onChangeNeedRestart({ secret: secretInput })}>确认</button>
            )}
            <div className="relative">
              <input
                className={`${inputCls} !w-52 !pr-8`}
                type={secretVisible ? "text" : "password"}
                value={secretInput}
                onChange={(e) => setSecretInput(e.target.value)}
              />
              <button className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-white cursor-pointer"
                onClick={() => setSecretVisible((p) => !p)}>
                {secretVisible ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
              </button>
            </div>
          </div>
        </SettingItem>

        {!!c?.["external-controller"] && (
          <>
            <SettingItem title="WebUI 面板">
              <Toggle v={externalUiEnabled} onChange={(v) => onChangeNeedRestart({ "external-ui": v ? "ui" : "" })} />
            </SettingItem>
            {externalUiEnabled && (
              <SettingItem title={<span className="inline-flex items-center gap-1">面板选择
                <button className="p-0.5 rounded hover:bg-white/10 cursor-pointer" title="更新面板" onClick={upgradeUi}>
                  <CloudDownload className={`w-3 h-3 text-slate-400 ${upgradingUi ? "animate-bounce" : ""}`} />
                </button>
                <button className="p-0.5 rounded hover:bg-white/10 cursor-pointer" title="打开面板" onClick={openExternalUi}>
                  <ExternalLink className="w-3 h-3 text-slate-400" />
                </button></span>}>
                <select
                  className="h-8 px-2 rounded-lg bg-white/10 border border-white/10 text-[11px] text-slate-200 cursor-pointer focus:outline-none"
                  value={externalUiUrl}
                  onChange={async (e) => {
                    await onChangeNeedRestart({ "external-ui-url": e.target.value });
                    upgradeUi();
                  }}
                >
                  {(WEBUI_PANELS.some((p) => p.url === externalUiUrl)
                    ? WEBUI_PANELS
                    : [{ name: "自定义面板", url: externalUiUrl }, ...WEBUI_PANELS]
                  ).map((p) => <option key={p.url} value={p.url} className="bg-slate-800">{p.name}</option>)}
                </select>
              </SettingItem>
            )}
          </>
        )}

        <SettingItem title="IPv6">
          <Toggle v={!!c?.ipv6} onChange={(v) => onChangeNeedRestart({ ipv6: v })} />
        </SettingItem>

        <SettingItem title="允许局域网连接">
          <Toggle v={!!c?.["allow-lan"]} onChange={(v) => onChangeNeedRestart({ "allow-lan": v })} />
        </SettingItem>
        {!!c?.["allow-lan"] && (
          <>
            <div className="py-2 border-b border-white/5">
              <h4 className="text-[12px] text-slate-300 font-semibold mb-1">允许连接的 IP 段</h4>
              {renderList(lanAllowed, setLanAllowed, "例: 192.168.1.0/24",
                c?.["lan-allowed-ips"] ?? DEFAULT_LAN_ALLOWED,
                () => onChangeNeedRestart({ "lan-allowed-ips": lanAllowed }))}
            </div>
            <div className="py-2 border-b border-white/5">
              <h4 className="text-[12px] text-slate-300 font-semibold mb-1">禁止连接的 IP 段</h4>
              {renderList(lanDisallowed, setLanDisallowed, "例: 192.168.1.0/24",
                c?.["lan-disallowed-ips"] ?? [],
                () => onChangeNeedRestart({ "lan-disallowed-ips": lanDisallowed }))}
            </div>
          </>
        )}

        {/* 用户验证（user:pass 双输入，复刻 authentication） */}
        <div className="py-2 border-b border-white/5">
          <div className="flex items-center justify-between mb-1">
            <h4 className="text-[12px] text-slate-300 font-semibold">用户验证</h4>
            {JSON.stringify(auth) !== JSON.stringify(c?.authentication ?? []) && (
              <button className={btnPrimary} onClick={() => onChangeNeedRestart({ authentication: auth })}>确认</button>
            )}
          </div>
          {[...auth, ""].map((a, index) => {
            const idx = a.indexOf(":");
            const user = idx >= 0 ? a.slice(0, idx) : a;
            const pass = idx >= 0 ? a.slice(idx + 1) : "";
            const upd = (u: string, p: string) => {
              const v = `${u}:${p}`;
              if (index === auth.length) setAuth([...auth, v]);
              else setAuth(auth.map((x, i) => (i === index ? v : x)));
            };
            return (
              <div key={index} className="flex mb-1.5 items-center gap-2">
                <input className={`${inputCls} !w-2/5`} placeholder="用户名" value={user} onChange={(e) => upd(e.target.value, pass)} />
                <span className="text-slate-500">:</span>
                <input className={inputCls} placeholder="密码" value={pass} onChange={(e) => upd(user, e.target.value)} />
                {index < auth.length && (
                  <button className={btnSec} onClick={() => setAuth(auth.filter((_, i) => i !== index))}>
                    <Trash2 className="w-3.5 h-3.5 text-amber-300" />
                  </button>
                )}
              </div>
            );
          })}
        </div>

        <div className="py-2 border-b border-white/5">
          <h4 className="text-[12px] text-slate-300 font-semibold mb-1">跳过验证前缀（前两项固定）</h4>
          {renderList(skipAuth, setSkipAuth, "例: 10.0.0.0/8",
            c?.["skip-auth-prefixes"] ?? DEFAULT_SKIP_AUTH,
            () => onChangeNeedRestart({ "skip-auth-prefixes": skipAuth }), true)}
        </div>

        <SettingItem title="使用 RTT 延迟测试 (unified-delay)">
          <Toggle v={!!c?.["unified-delay"]} onChange={(v) => onChangeNeedRestart({ "unified-delay": v })} />
        </SettingItem>
        <SettingItem title="TCP 并发">
          <Toggle v={!!c?.["tcp-concurrent"]} onChange={(v) => onChangeNeedRestart({ "tcp-concurrent": v })} />
        </SettingItem>
        <SettingItem title="记住选择的节点 (store-selected)">
          <Toggle v={!!c?.profile?.["store-selected"]} onChange={(v) => onChangeNeedRestart({ profile: { "store-selected": v } })} />
        </SettingItem>
        <SettingItem title="记住 FakeIP 映射 (store-fake-ip)">
          <Toggle v={!!c?.profile?.["store-fake-ip"]} onChange={(v) => onChangeNeedRestart({ profile: { "store-fake-ip": v } })} />
        </SettingItem>

        <SettingItem title="日志等级">
          <select
            className="h-8 px-2 rounded-lg bg-white/10 border border-white/10 text-[11px] text-slate-200 cursor-pointer focus:outline-none"
            value={c?.["log-level"] ?? "info"}
            onChange={(e) => onChangeNeedRestart({ "log-level": e.target.value })}
          >
            {["silent", "error", "warning", "info", "debug"].map((l) => (
              <option key={l} value={l} className="bg-slate-800">{l}</option>
            ))}
          </select>
        </SettingItem>
        <SettingItem title="查找进程 (find-process-mode)" divider={false}>
          <select
            className="h-8 px-2 rounded-lg bg-white/10 border border-white/10 text-[11px] text-slate-200 cursor-pointer focus:outline-none"
            value={c?.["find-process-mode"] ?? "strict"}
            onChange={(e) => onChangeNeedRestart({ "find-process-mode": e.target.value })}
          >
            {["strict", "off", "always"].map((l) => (
              <option key={l} value={l} className="bg-slate-800">{l}</option>
            ))}
          </select>
        </SettingItem>
      </div>

      {/* 进程与配置生成（对齐 clash-party 内核进程 / 配置工厂相关设置） */}
      <div className={`${cardCls} p-4`}>
        <h3 className="text-sm font-bold text-white mb-1">进程与配置</h3>

        <SettingItem title="内核 CPU 优先级">
          <select
            className="h-8 px-2 rounded-lg bg-white/10 border border-white/10 text-[11px] text-slate-200 cursor-pointer focus:outline-none"
            value={app?.cpuPriority ?? "NORMAL_PRIORITY_CLASS"}
            onChange={(e) => patchApp({ cpuPriority: e.target.value }, true)}
          >
            {[
              ["REALTIME_PRIORITY_CLASS", "实时"],
              ["HIGH_PRIORITY_CLASS", "高"],
              ["ABOVE_NORMAL_PRIORITY_CLASS", "高于正常"],
              ["NORMAL_PRIORITY_CLASS", "正常"],
              ["BELOW_NORMAL_PRIORITY_CLASS", "低于正常"],
              ["IDLE_PRIORITY_CLASS", "低"],
            ].map(([v, l]) => <option key={v} value={v} className="bg-slate-800">{l}</option>)}
          </select>
        </SettingItem>

        <SettingItem title={<span title="启动前用 mihomo -t 预校验配置，可提前发现错误">启动前校验配置</span>}>
          <Toggle
            v={app?.testProfileOnStart !== false}
            onChange={(v) => patchApp({ testProfileOnStart: v })}
          />
        </SettingItem>

        <SettingItem title={<span title="每个订阅使用独立的内核工作目录，避免缓存互相污染">独立工作目录</span>}>
          <Toggle v={!!app?.diff_work_dir} onChange={(v) => patchApp({ diff_work_dir: v }, true)} />
        </SettingItem>

        <SettingItem title={<span title="关闭后生成配置时会移除 dns.nameserver-policy">应用 nameserver-policy</span>}>
          <Toggle
            v={app?.use_nameserver_policy !== false}
            onChange={(v) => patchApp({ use_nameserver_policy: v }, true)}
          />
        </SettingItem>

        <SettingItem title="崩溃后自动拉起内核">
          <Toggle v={!!app?.keep_profile_alive} onChange={(v) => patchApp({ keep_profile_alive: v })} />
        </SettingItem>

        <SettingItem title="配置维护" divider={false}>
          <div className="flex items-center gap-2">
            <button
              className={btnSec}
              title="不重启进程，直接让内核重新加载配置文件"
              onClick={async () => {
                try { await mihomoApi.hotReloadConfig(); setMsg("已热重载配置"); }
                catch (e: any) { setMsg(`热重载失败: ${e}`); }
              }}
            >
              热重载配置
            </button>
            <button
              className={btnSec}
              title="清空 Smart 内核的学习缓存"
              onClick={async () => {
                try { await mihomoApi.smartFlushCache(); setMsg("已清空 Smart 缓存"); }
                catch (e: any) { setMsg(`清空失败: ${e}`); }
              }}
            >
              清空 Smart 缓存
            </button>
          </div>
        </SettingItem>
      </div>
    </div>
  );
}
