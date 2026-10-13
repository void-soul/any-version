import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { useProjects, type SkillProject } from "./useProjects";
import { useTranslation } from "react-i18next";
import {
  Plug,
  Plus,
  Trash2,
  RefreshCw,
  Server,
  Edit3,
  Link2,
  Check,
  X,
  Search,
  FolderOpen,
  ChevronDown,
} from "lucide-react";
import { alertError, theamedAlert } from "../shared/ThemedAlert";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";

interface McpServer {
  id: string;
  name: string;
  transport: string; // stdio | http | sse
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  url: string;
  headers: Record<string, string>;
  enabled: boolean;
  enabledTools: string[];
  description: string | null;
  installMethod: string;
}

/** 项目里已装的 MCP（后端 list_project_mcps） */
interface ProjectMcp {
  /** 在 `<项目>/.mcp.json` 里的键名 */
  name: string;
  transport: string;
  command: string;
  args: string[];
  url: string;
  /** 仓库里有同名定义 = 受管；否则是用户手写的「项目自带」 */
  managed: boolean;
}

interface DiscoveredMcp {
  toolId: string;
  name: string;
  transport: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  url: string;
  headers: Record<string, string>;
  alreadyManaged: boolean;
}

/** MCP 服务器预设：只用于预填表单，不直接落库（命令/地址用户通常还要改）。 */
interface McpPreset {
  id: string;
  name: string;
  description: string;
  transport: string;
  command: string;
  args: string[];
  url: string;
  /** 依赖的本地服务 id（如 wigolo）；空表示无依赖 */
  serviceId: string;
}

// 将 "KEY=VALUE" 多行文本解析为对象
function parseKV(text: string): Record<string, string> {
  const obj: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const idx = t.indexOf("=");
    if (idx <= 0) continue;
    const k = t.slice(0, idx).trim();
    const v = t.slice(idx + 1).trim();
    if (k) obj[k] = v;
  }
  return obj;
}

// 将对象序列化为 "KEY=VALUE" 多行文本
function serializeKV(obj: Record<string, string>): string {
  return Object.entries(obj).map(([k, v]) => `${k}=${v}`).join("\n");
}

const TRANSPORT_KEY: Record<string, string> = {
  stdio: "transportStdio",
  http: "http",
  sse: "sse",
};

export default function McpManager() {
  const { t } = useTranslation();
  const [servers, setServers] = useState<McpServer[]>([]);
  /** 全局配置里现存的 MCP（一次性清理用；只读列举） */
  const [globalList, setGlobalList] = useState<DiscoveredMcp[]>([]);
  /** 读取全局配置失败的原因。刻意**不吞**：之前 `.catch(() => [])` 让整块 UI 静默消失，
   *  用户看到的是「没有需要迁移的东西」，而不是「读取失败」——两者天差地别。 */
  const [globalErr, setGlobalErr] = useState<string | null>(null);
  /** 勾选要收进仓库的全局条目 */
  const [pickedGlobal, setPickedGlobal] = useState<Set<string>>(new Set());
  const [migrating, setMigrating] = useState(false);
  /** 项目维度：MCP 按项目安装，不再全局部署。
   *  项目清单与技能页**共用同一个 hook**（./useProjects）—— 否则在技能页新加的项目
   *  这里不会刷新，两边会各看到一份不同的项目列表。 */
  const {
    projects,
    selProject,
    setSelProject,
    picked: pickedProjects,
    setPicked: setPickedProjects,
    removeProject: removeProjectShared,
  } = useProjects();
  const [projMcps, setProjMcps] = useState<ProjectMcp[]>([]);
  const [projBusy, setProjBusy] = useState<string>("");
  /** 仓库区折叠（与技能安装页的「技能库」通栏一致） */
  const [repoCollapsed, setRepoCollapsed] = useState(false);
  /** 各项目已装数量（左侧列表上的数字；只拉当前项目的详情太慢，这里各存一份计数） */
  const [projCounts, setProjCounts] = useState<Record<string, number>>({});
  /** 危险操作二次确认（删项目等） */
  const [confirmReq, setConfirmReq] = useState<ConfirmRequest | null>(null);

  /**
   * 删除项目条目：二次确认走共享弹框。
   *
   * 删的只是「清单里的一项」——项目目录里的 `.mcp.json`、已装技能一概不动。
   * 走共享 store 的 removeProject，所以技能页那边也会立刻同步（两页读同一份快照）。
   */
  const askRemoveProject = (p: SkillProject) => {
    setConfirmReq({
      title: t("skillmgr.projRemove"),
      desc: t("skillmgr.projRemoveConfirm", { name: p.name || p.path }),
      confirmText: t("common.delete"),
      danger: true,
      onConfirm: async () => {
        try {
          await removeProjectShared(p.id);
        } catch (e) {
          alertError(String(e));
        }
      },
    });
  };
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  // 表单状态
  const [fName, setFName] = useState("");
  const [fTransport, setFTransport] = useState("stdio");
  const [fCommand, setFCommand] = useState("");
  const [fArgs, setFArgs] = useState("");
  const [fEnv, setFEnv] = useState("");
  const [fCwd, setFCwd] = useState("");
  const [fUrl, setFUrl] = useState("");
  const [fHeaders, setFHeaders] = useState("");
  const [fEnabled, setFEnabled] = useState(true);
  const [fDescription, setFDescription] = useState("");
  const [saving, setSaving] = useState(false);
  // MCP 服务器预设（后端静态清单，只在挂载时取一次）
  const [presets, setPresets] = useState<McpPreset[]>([]);
  useEffect(() => {
    invoke<McpPreset[]>("get_mcp_presets")
      .then(setPresets)
      .catch(() => {});
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [srv, glob] = await Promise.all([
        invoke<McpServer[]>("get_mcp_servers"),
        invoke<DiscoveredMcp[]>("list_global_mcp").catch((e) => { setGlobalErr(String(e)); return [] as DiscoveredMcp[]; }),
      ]);
      setServers(srv);
      // 已经收进仓库的条目不再重复提示迁移
      setGlobalList(glob.filter((d) => !d.alreadyManaged));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  /**
   * 各项目的已装数量（左侧列表上的数字）。
   *
   * 刻意**不放在 `load` 里**：那个 useCallback 的依赖是 `[]`，闭包捕获的是首次渲染的
   * `projects = []`，于是计数一次都没算出来过——徽章永远不显示，连刷新按钮都救不回来
   * （刷新只重跑 `load`）。做成依赖共享 store 的 `projects` 的回调，才既正确、
   * 又能跟着项目增删一起刷新（在技能页删项目，这里也会重算），刷新按钮也能复用。
   *
   * 成本：每个项目读一次 `.mcp.json`（每人一个小 JSON），并发发出。
   */
  const reloadProjCounts = useCallback(async () => {
    await Promise.all(
      projects.map(async (p) => {
        try {
          const list = await invoke<ProjectMcp[]>("list_project_mcps", { projectId: p.id });
          setProjCounts((m) => ({ ...m, [p.id]: list.length }));
        } catch {
          // 项目文件解析失败：数字留空即可，详情页会如实报错，不在这里打断整页
        }
      }),
    );
  }, [projects]);

  useEffect(() => { void reloadProjCounts(); }, [reloadProjCounts]);

  /// 读当前项目的 `.mcp.json`
  const loadProjMcps = useCallback(async (pid: string) => {
    if (!pid) {
      setProjMcps([]);
      return;
    }
    try {
      setProjMcps(await invoke<ProjectMcp[]>("list_project_mcps", { projectId: pid }));
    } catch (e: any) {
      // 项目文件解析失败必须如实报错，不能当成「没装任何 MCP」——
      // 后者会让用户以为功能坏了，而文件里其实有配置
      setProjMcps([]);
      alertError(String(e));
    }
  }, []);

  useEffect(() => { void loadProjMcps(selProject); }, [selProject, loadProjMcps]);

  const resetForm = () => {
    setEditingId(null);
    setFName("");
    setFTransport("stdio");
    setFCommand("");
    setFArgs("");
    setFEnv("");
    setFCwd("");
    setFUrl("");
    setFHeaders("");
    setFEnabled(true);
    setFDescription("");
  };

  const openAdd = () => {
    resetForm();
    setShowForm(true);
  };

  /// 从预设预填表单（编辑态一并清掉，避免预填内容覆盖到另一个条目上）
  const applyPreset = (p: McpPreset) => {
    setEditingId(null);
    setFName(p.name);
    setFTransport(p.transport);
    setFCommand(p.command);
    setFArgs(p.args.join(" "));
    setFUrl(p.url);
    setFDescription(p.description);
    setFEnv("");
    setFCwd("");
    setFHeaders("");
    setFEnabled(true);
    setShowForm(true);
  };

  const openEdit = (s: McpServer) => {
    setEditingId(s.id);
    setFName(s.name);
    setFTransport(s.transport);
    setFCommand(s.command);
    setFArgs(s.args.join("\n"));
    setFEnv(serializeKV(s.env));
    setFCwd(s.cwd ?? "");
    setFUrl(s.url);
    setFHeaders(serializeKV(s.headers));
    setFEnabled(s.enabled);
    setFDescription(s.description ?? "");
    setShowForm(true);
  };

  const handleSave = async () => {
    if (!fName.trim()) { theamedAlert(t("mcp.needName")); return; }
    if (fTransport === "stdio") {
      if (!fCommand.trim()) { theamedAlert(t("mcp.needCommand")); return; }
    } else if (!fUrl.trim()) {
      theamedAlert(t("mcp.needUrl")); return;
    }
    const payload: McpServer = {
      id: editingId ?? "",
      name: fName.trim(),
      transport: fTransport,
      command: fCommand.trim(),
      args: fArgs.split("\n").map((x) => x.trim()).filter(Boolean),
      env: parseKV(fEnv),
      cwd: fCwd.trim() || null,
      url: fUrl.trim(),
      headers: parseKV(fHeaders),
      enabled: fEnabled,
      enabledTools: editingId
        ? (servers.find((s) => s.id === editingId)?.enabledTools ?? [])
        : [],
      description: fDescription.trim() || null,
      installMethod: "managed",
    };
    setSaving(true);
    try {
      await invoke("save_mcp_server", { server: payload });
      setShowForm(false);
      resetForm();
      await load();
    } catch (e: any) {
      alertError(t("mcp.saveFail", { err: String(e) }));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm(t("mcp.confirmDelete"))) return;
    try {
      await invoke("delete_mcp_server", { id });
      await load();
    } catch (e: any) { alertError(t("mcp.deleteFail", { err: String(e) })); }
  };

  /// 装给当前项目 / 从当前项目移除
  const installToProject = async (id: string) => {
    // 没勾选项目时退回「当前正在看的那个」，免得按钮点了没反应
    const targets = pickedProjects.size > 0 ? [...pickedProjects] : selProject ? [selProject] : [];
    if (targets.length === 0) return;
    const names = targets.map((pid) => projects.find((p) => p.id === pid)?.name || pid);
    // 多个项目时把清单摆出来：装到哪几个是**会写项目文件**的操作，
    // 装错项目不会有人报错，只会在那个项目里多出一个没人认领的 .mcp.json
    if (targets.length > 1 && !confirm(t("mcp.installConfirmProjects", { list: names.join("、") }))) return;
    setProjBusy(id);
    try {
      for (const pid of targets) {
        await invoke("install_mcp_to_project", { projectId: pid, serverId: id });
      }
      await loadProjMcps(selProject);
      // 左侧徽章是**每个项目各一个数字**，而刚才装的可能不是当前项目，所以整批重算
      void reloadProjCounts();
      void load();
    } catch (e: any) {
      alertError(t("mcp.opFail", { err: String(e) }));
    } finally {
      setProjBusy("");
    }
  };

  const removeFromProject = async (name: string, managed: boolean) => {
    if (!selProject) return;
    // 项目自带（手写）的条目删掉用户会心疼，必须说清这是什么
    if (!managed && !confirm(t("mcp.removeLocalConfirm", { name }))) return;
    setProjBusy(name);
    try {
      await invoke("remove_mcp_from_project", { projectId: selProject, name });
      await loadProjMcps(selProject);
      void reloadProjCounts();
    } catch (e: any) {
      alertError(t("mcp.opFail", { err: String(e) }));
    } finally {
      setProjBusy("");
    }
  };

  /// 一次性迁移：把勾选的全局条目收进仓库，并从全局配置文件里删掉它们
  const runMigrate = async () => {
    if (pickedGlobal.size === 0) return;
    const msg = t("mcp.migrateConfirm", { n: pickedGlobal.size });
    if (!confirm(msg)) return;
    setMigrating(true);
    try {
      const r = await invoke<{ imported: string[]; filesUpdated: number; failed: string[] }>(
        "migrate_global_mcp",
        { names: [...pickedGlobal] },
      );
      setPickedGlobal(new Set());
      await load();
      if (r.failed.length > 0) {
        alertError(t("mcp.migratePartial", { failed: r.failed.join("；") }));
      }
    } catch (e: any) {
      alertError(t("mcp.opFail", { err: String(e) }));
    } finally {
      setMigrating(false);
    }
  };

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-slate-500">
        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
        <span className="text-body">{t("mcp.loading")}</span>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-6 space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">{t("mcp.title")}</h3>
          
        </div>
        <div className="flex gap-2">
          <button
            onClick={load}
            className="px-2.5 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-tiny text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <RefreshCw className="w-3 h-3" /> {t("mcp.refresh")}
          </button>
        </div>
      </div>

      {/* 添加 / 编辑表单 */}
      {showForm && (
        <div className="p-3 rounded-card bg-slate-900/40 border border-[var(--module-accent-ring)] space-y-3">
          <div className="flex items-center justify-between">
            <h4 className="text-body font-bold text-[var(--module-accent)] flex items-center gap-1.5">
              <Plug className="w-3.5 h-3.5" /> {editingId ? t("mcp.editServer") : t("mcp.addMcpServer")}
            </h4>
            <button onClick={() => { setShowForm(false); resetForm(); }} className="text-slate-500 hover:text-slate-300 cursor-pointer">
              <X className="w-4 h-4" />
            </button>
          </div>

          {/* 从预设添加：只预填表单，落库/改动仍由用户确认 */}
          {!editingId && presets.length > 0 && (
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-tiny text-slate-500">{t("mcp.fromPreset")}</span>
              {presets.map((preset) => {
                const used = servers.some((s) => s.name === preset.name);
                return (
                  <button
                    key={preset.id}
                    onClick={() => applyPreset(preset)}
                    title={used ? `${preset.description}${t("mcp.presetAlreadyAdded")}` : preset.description}
                    className={`px-2 py-0.5 rounded-md text-tiny bg-white/5 border border-white/10 cursor-pointer transition-all hover:text-white hover:border-[var(--module-accent-ring)] ${
                      used ? "text-slate-600" : "text-slate-300"
                    }`}
                  >
                    {preset.name}
                  </button>
                );
              })}
            </div>
          )}

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-micro text-slate-500 block mb-1">{t("mcp.nameLabel")}</label>
              <input
                value={fName}
                onChange={(e) => setFName(e.target.value)}
                placeholder={t("mcp.namePh")}
                className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
              />
            </div>
            <div>
              <label className="text-micro text-slate-500 block mb-1">{t("mcp.transportLabel")}</label>
              <select
                value={fTransport}
                onChange={(e) => setFTransport(e.target.value)}
                className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
              >
                <option value="stdio">{t("mcp.transportStdio")}</option>
                <option value="http">HTTP</option>
                <option value="sse">SSE</option>
              </select>
            </div>
          </div>

          {fTransport === "stdio" ? (
            <>
              <div>
                <label className="text-micro text-slate-500 block mb-1">{t("mcp.commandLabel")}</label>
                <input
                  value={fCommand}
                  onChange={(e) => setFCommand(e.target.value)}
                  placeholder={t("mcp.commandPh")}
                  className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
                />
              </div>
              <div>
                <label className="text-micro text-slate-500 block mb-1">{t("mcp.argsLabel")}</label>
                <textarea
                  value={fArgs}
                  onChange={(e) => setFArgs(e.target.value)}
                  rows={2}
                  placeholder={"-y\n@modelcontextprotocol/server-everything"}
                  className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)] resize-none"
                />
              </div>
              <div>
                <label className="text-micro text-slate-500 block mb-1">{t("mcp.cwdLabel")}</label>
                <input
                  value={fCwd}
                  onChange={(e) => setFCwd(e.target.value)}
                  placeholder={t("mcp.cwdPh")}
                  className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
                />
              </div>
              <div>
                <label className="text-micro text-slate-500 block mb-1">{t("mcp.envLabel")}</label>
                <textarea
                  value={fEnv}
                  onChange={(e) => setFEnv(e.target.value)}
                  rows={2}
                  placeholder={"API_KEY=xxx"}
                  className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)] resize-none"
                />
              </div>
            </>
          ) : (
            <>
              <div>
                <label className="text-micro text-slate-500 block mb-1">URL</label>
                <input
                  value={fUrl}
                  onChange={(e) => setFUrl(e.target.value)}
                  placeholder={t("mcp.urlPh")}
                  className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
                />
              </div>
              <div>
                <label className="text-micro text-slate-500 block mb-1">{t("mcp.headersLabel")}</label>
                <textarea
                  value={fHeaders}
                  onChange={(e) => setFHeaders(e.target.value)}
                  rows={2}
                  placeholder={"Authorization=Bearer xxx"}
                  className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)] resize-none"
                />
              </div>
            </>
          )}

          <div>
            <label className="text-micro text-slate-500 block mb-1">{t("mcp.descLabel")}</label>
            <input
              value={fDescription}
              onChange={(e) => setFDescription(e.target.value)}
              placeholder={t("mcp.descPh")}
              className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
            />
          </div>

          <label className="flex items-center gap-2 text-tiny text-slate-400 cursor-pointer">
            <input type="checkbox" checked={fEnabled} onChange={(e) => setFEnabled(e.target.checked)} className="accent-[var(--module-accent)]" />
            {t("mcp.globalEnable")}
          </label>

          <div className="flex justify-end gap-2 pt-1">
            <button
              onClick={() => { setShowForm(false); resetForm(); }}
              className="px-3 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-tiny text-slate-400 hover:text-white cursor-pointer"
            >
              {t("mcp.cancel")}
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="px-4 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-40 text-white text-tiny font-semibold cursor-pointer flex items-center gap-1"
            >
              {saving ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
              {saving ? t("mcp.saving") : t("mcp.saveDeploy")}
            </button>
          </div>
        </div>
      )}

      {/* 顶部通栏：MCP 仓库（可折叠）。折叠后仍看得见条数、也照样点得了添加 */}
      <div className="rounded-card bg-white/[0.02] border border-white/10 p-3 space-y-2">
        <div className="flex items-center justify-between gap-2">
          <button
            onClick={() => setRepoCollapsed((v) => !v)}
            className="flex items-center gap-1.5 cursor-pointer min-w-0"
            title={repoCollapsed ? t("mcp.repoExpand") : t("mcp.repoCollapse")}
          >
            <ChevronDown className={`w-3.5 h-3.5 text-slate-400 flex-shrink-0 transition-transform ${repoCollapsed ? "" : "rotate-180"}`} />
            <span className="text-body font-bold text-slate-300">{t("mcp.repoTitle")}</span>
            <span className="text-micro text-slate-500 truncate">{servers.length}</span>
          </button>
          {/* 全站只保留这一个「添加服务器」入口：仓库就是唯一入口，
              再在页头放一个只会让人以为「添加」有区别（加到仓库 vs 加到全局） */}
          <button
            onClick={openAdd}
            className="flex items-center gap-1 px-2.5 py-1 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-tiny font-semibold cursor-pointer flex-shrink-0"
          >
            <Plus className="w-3 h-3" /> {t("mcp.addServer")}
          </button>
        </div>

      {!repoCollapsed && (
        <>
      {/* 已安装服务器列表 */}
      {servers.length === 0 ? (
        <div className="h-48 border border-dashed border-white/5 rounded-panel flex flex-col items-center justify-center text-slate-500">
          <Server className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-body font-bold text-slate-400">{t("mcp.noServers")}</span>
          <span className="text-tiny text-slate-600 mt-1">{t("mcp.noServersHint")}</span>
        </div>
      ) : (
        <div className="space-y-2">
          {servers.map((s) => {
            const installed = projMcps.find((p) => p.managed && p.name === s.name);
            const busy = projBusy === s.id;
            return (
              /* 单行紧凑：与技能库列表同一套节奏。原先每条一张p-4 卡片 + 三行，
                 六个 MCP 就要占满整屏，真正的信息（名字 / 传输方式 / 装没装）反而被摊薄。 */
              <div
                key={s.id}
                className="flex items-center gap-1.5 px-1.5 py-1 rounded-ctl bg-white/[0.03] hover:bg-white/[0.06] transition-colors"
              >
                <input
                  type="checkbox"
                  checked={!!installed}
                  disabled={busy || (!selProject && pickedProjects.size === 0)}
                  onChange={() => (installed ? removeFromProject(s.name, true) : installToProject(s.id))}
                  title={installed
                    ? t("mcp.installedIn", { name: projects.find((p) => p.id === selProject)?.name ?? "" })
                    : t("mcp.installToProject")}
                  className="w-3 h-3 accent-[var(--module-accent)] flex-shrink-0 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                />
                <Plug className="w-3 h-3 text-[var(--module-accent)] flex-shrink-0" />
                <span className="text-tiny font-semibold text-slate-200 truncate flex-shrink-0 max-w-[38%]">
                  {s.name}
                </span>
                <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-400 flex-shrink-0">
                  {t(`mcp.${TRANSPORT_KEY[s.transport] || s.transport}`) || s.transport}
                </span>
                <span className="text-micro text-slate-500 truncate flex-1 font-mono" title={s.transport === "stdio" ? `${s.command} ${s.args.join(" ")}`.trim() : s.url}>
                  {s.transport === "stdio" ? `${s.command} ${s.args.join(" ")}`.trim() : s.url}
                </span>
                <button
                  onClick={() => openEdit(s)}
                  className="text-slate-600 hover:text-[var(--module-accent)] cursor-pointer flex-shrink-0"
                  title={t("mcp.edit")}
                >
                  <Edit3 className="w-3 h-3" />
                </button>
                <button
                  onClick={() => handleDelete(s.id)}
                  className="text-slate-600 hover:text-red-400 cursor-pointer flex-shrink-0"
                  title={t("mcp.delete")}
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            );
          })}
        </div>
      )}
        </>
      )}
      </div>

      {/* 下：左项目列表（勾选 = 安装目标）/ 右项目内已装 MCP。与技能安装页同构。 */}
      <div className="grid grid-cols-1 lg:grid-cols-[264px_1fr] gap-3 items-start">
        {/* 左：项目列表 */}
        <div className="rounded-card bg-white/[0.02] border border-white/10 p-2 space-y-1.5">
          <div className="flex items-center justify-between px-1">
            <span className="text-body font-bold text-slate-300">{t("mcp.projectListTitle")}</span>
            <div className="flex items-center gap-1">
              {/* 全选：往多个项目装同一批 MCP 时最烦的就是一个个点 */}
              {projects.length > 1 && (
                <button
                  onClick={() => setPickedProjects((prev) =>
                    prev.size === projects.length ? new Set() : new Set(projects.map((p) => p.id)),
                  )}
                  className="px-1.5 py-0.5 rounded text-micro bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer"
                  title={t("mcp.projectSelectAll")}
                >
                  {pickedProjects.size === projects.length ? t("skillmgr.projUnselectAll") : t("skillmgr.projSelectAll")}
                </button>
              )}
              <button
                onClick={() => { void load(); void reloadProjCounts(); }}
                className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-slate-200 cursor-pointer"
                title={t("mcp.refresh")}
              >
                <RefreshCw className="w-3 h-3" />
              </button>
            </div>
          </div>

          {projects.length === 0 ? (
            <p className="text-tiny text-slate-500 px-1 py-2">{t("mcp.noProjects")}</p>
          ) : (
            projects.map((p) => {
              const viewing = selProject === p.id;
              const n = projCounts[p.id] ?? 0;
              return (
                <div
                  key={p.id}
                  className={`flex items-start gap-1.5 px-1.5 py-1.5 rounded-ctl border transition-colors ${
                    viewing
                      ? "border-[var(--module-accent-ring)] bg-[var(--module-accent-soft)]"
                      : "border-transparent hover:bg-white/5"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={pickedProjects.has(p.id)}
                    onChange={() => setPickedProjects((prev) => {
                      const next = new Set(prev);
                      next.has(p.id) ? next.delete(p.id) : next.add(p.id);
                      return next;
                    })}
                    title={t("skillmgr.projPickAsTarget")}
                    className="w-3 h-3 mt-0.5 accent-[var(--module-accent)] flex-shrink-0 cursor-pointer"
                  />
                  <button onClick={() => setSelProject(p.id)} className="flex-1 min-w-0 text-left cursor-pointer">
                    <div className="flex items-center gap-1.5">
                      <span className="text-tiny font-semibold text-slate-200 truncate flex-1">
                        {p.name || p.path}
                      </span>
                      {/* 数量 0 也显示：`没装过` 与 `还没算出来` 必须能一眼分开，
                          藏起来就退化成「不知道有没有装」 */}
                      <span
                        className={`px-1.5 py-0.5 rounded text-micro font-semibold flex-shrink-0 ${
                          n > 0 ? "bg-emerald-500/15 text-emerald-400" : "bg-white/5 text-slate-500"
                        }`}
                        title={t("mcp.projMcpCountHint", { n })}
                      >
                        {n}
                      </span>
                    </div>
                    <div className="flex items-center gap-1">
                      <div className="text-micro text-slate-500 font-mono truncate flex-1">{p.path}</div>
                      {/* 打开项目目录：装完想去项目里看一眼 .mcp.json 长什么样 */}
                      <button
                        onClick={() => void openPath(p.path).catch((e) => alertError(String(e)))}
                        className="text-slate-600 hover:text-[var(--module-accent)] cursor-pointer flex-shrink-0"
                        title={t("mcp.projectOpenDir")}
                      >
                        <FolderOpen className="w-3 h-3" />
                      </button>
                    </div>
                  </button>
                  {/* 删除项目条目（只删清单记录，项目目录里的 .mcp.json 一概不动） */}
                  <button
                    onClick={() => askRemoveProject(p)}
                    className="text-slate-600 hover:text-red-400 cursor-pointer flex-shrink-0 mt-0.5"
                    title={t("skillmgr.projRemove")}
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              );
            })
          )}
        </div>

        {/* 右：当前项目内已装的 MCP */}
        <div className="min-w-0 rounded-card bg-white/[0.02] border border-white/10 p-3 space-y-2">
          {!selProject ? (
            <p className="text-tiny text-slate-500 py-6 text-center">{t("mcp.pickProject")}</p>
          ) : (
            <>
              <div className="flex items-center justify-between gap-2">
                <span className="text-body font-bold text-slate-300 truncate">
                  {t("mcp.projectInstalledTitle")}
                  <span className="ml-1.5 text-micro text-slate-500 font-normal font-mono">
                    {projects.find((p) => p.id === selProject)?.path}
                  </span>
                </span>
                <span className="text-micro text-slate-500 flex-shrink-0">
                  {t("mcp.projectFileHint", { name: ".mcp.json" })}
                </span>
              </div>

              {projMcps.length === 0 ? (
                <p className="text-tiny text-slate-500 py-4 text-center">{t("mcp.projectNone")}</p>
              ) : (
                <div className="space-y-1">
                  {projMcps.map((m) => (
                    <div key={m.name} className="flex items-center gap-2 rounded-ctl bg-slate-900/40 border border-white/5 px-2.5 py-1.5">
                      <Plug className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" />
                      <span className="text-tiny font-bold text-slate-200 font-mono truncate">{m.name}</span>
                      <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-400 flex-shrink-0">
                        {t(`mcp.${TRANSPORT_KEY[m.transport] || m.transport}`) || m.transport}
                      </span>
                      {!m.managed && (
                        <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-white/5 text-slate-400 flex-shrink-0"
                          title={t("mcp.localHint")}>
                          {t("mcp.fromProject")}
                        </span>
                      )}
                      <span className="text-micro text-slate-500 truncate flex-1 font-mono">
                        {m.url || `${m.command} ${m.args.join(" ")}`.trim()}
                      </span>
                      <button
                        onClick={() => removeFromProject(m.name, m.managed)}
                        disabled={projBusy === m.name}
                        className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all disabled:opacity-40 flex-shrink-0"
                        title={t("mcp.removeFromProject")}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
      </div>

      {/* 一次性清理：全局配置里的 MCP 对所有项目生效，是上下文被污染的根源 */}
      {globalErr && (
        <div className="p-2.5 rounded-ctl bg-red-500/10 text-red-400 text-tiny break-all">
          {t("mcp.globalReadFail", { err: globalErr })}
        </div>
      )}

      {globalList.length > 0 && (
        <div className="p-3 rounded-card bg-amber-500/5 border border-amber-500/20 space-y-2">
          <div className="flex items-center justify-between gap-2">
            <span className="text-tiny font-bold text-amber-300 flex items-center gap-1.5">
              <Search className="w-3.5 h-3.5" /> {t("mcp.globalTitle", { count: globalList.length })}
            </span>
            <button
              onClick={() => void runMigrate()}
              disabled={pickedGlobal.size === 0 || migrating}
              className="px-2.5 py-1 rounded-md text-micro font-semibold bg-amber-600 hover:bg-amber-500 text-white cursor-pointer transition-all flex items-center gap-1 disabled:opacity-40"
            >
              {migrating ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Link2 className="w-3 h-3" />}
              {t("mcp.migrate", { n: pickedGlobal.size })}
            </button>
          </div>
          <p className="text-micro text-slate-500">{t("mcp.globalHint")}</p>
          <div className="space-y-1">
            {globalList.map((d) => {
              const on = pickedGlobal.has(d.name);
              return (
                <button
                  key={`${d.toolId}:${d.name}`}
                  onClick={() => setPickedGlobal((p) => {
                    const next = new Set(p);
                    on ? next.delete(d.name) : next.add(d.name);
                    return next;
                  })}
                  className={`w-full flex items-center gap-2 rounded-ctl border px-2.5 py-1.5 cursor-pointer transition-all text-left ${
                    on ? "bg-amber-500/10 border-amber-500/30" : "bg-slate-900/40 border-white/5 hover:border-white/15"
                  }`}
                >
                  <span className={`w-3 h-3 rounded-sm flex-shrink-0 border ${on ? "bg-amber-400 border-amber-300" : "border-white/20"}`} />
                  <span className="text-tiny font-bold text-slate-200 font-mono truncate">{d.name}</span>
                  <span className="text-micro text-slate-500 truncate flex-1 font-mono">
                    {d.url || `${d.command} ${d.args.join(" ")}`.trim()}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* 危险操作二次确认（删项目等） */}
      <ConfirmDialogHost request={confirmReq} onClose={() => setConfirmReq(null)} />
      </div>
  );
}
