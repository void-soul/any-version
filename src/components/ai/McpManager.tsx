import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
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
} from "lucide-react";
import { alertError, theamedAlert } from "../shared/ThemedAlert";

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

/** 项目（复用技能那边同一个 projects.json，后端 list_projects） */
interface SkillProject {
  id: string;
  path: string;
  name: string;
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
  /** 勾选要收进仓库的全局条目 */
  const [pickedGlobal, setPickedGlobal] = useState<Set<string>>(new Set());
  const [migrating, setMigrating] = useState(false);
  /** 项目维度：MCP 按项目安装，不再全局部署 */
  const [projects, setProjects] = useState<SkillProject[]>([]);
  const [selProject, setSelProject] = useState<string>("");
  const [projMcps, setProjMcps] = useState<ProjectMcp[]>([]);
  const [projBusy, setProjBusy] = useState<string>("");
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
      const [srv, glob, projs] = await Promise.all([
        invoke<McpServer[]>("get_mcp_servers"),
        invoke<DiscoveredMcp[]>("list_global_mcp").catch(() => [] as DiscoveredMcp[]),
        invoke<SkillProject[]>("list_projects").catch(() => [] as SkillProject[]),
      ]);
      setServers(srv);
      // 已经收进仓库的条目不再重复提示迁移
      setGlobalList(glob.filter((d) => !d.alreadyManaged));
      setProjects(projs);
      setSelProject((cur) => (cur && projs.some((p) => p.id === cur) ? cur : projs[0]?.id ?? ""));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

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
    if (!selProject) return;
    setProjBusy(id);
    try {
      await invoke("install_mcp_to_project", { projectId: selProject, serverId: id });
      await loadProjMcps(selProject);
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
          <p className="text-tiny text-slate-500 mt-0.5">{t("mcp.subtitle")}</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={load}
            className="px-2.5 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-tiny text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <RefreshCw className="w-3 h-3" /> {t("mcp.refresh")}
          </button>
          <button
            onClick={openAdd}
            className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-tiny font-semibold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-[var(--module-accent-ring)]"
          >
            <Plus className="w-3 h-3" /> {t("mcp.addServer")}
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

      {/* 已安装服务器列表 */}
      {servers.length === 0 ? (
        <div className="h-48 border border-dashed border-white/5 rounded-panel flex flex-col items-center justify-center text-slate-500">
          <Server className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-body font-bold text-slate-400">{t("mcp.noServers")}</span>
          <span className="text-tiny text-slate-600 mt-1">{t("mcp.noServersHint")}</span>
        </div>
      ) : (
        <div className="space-y-2">
          {servers.map((s) => (
            <div key={s.id} className="rounded-card bg-slate-900/30 border border-white/5 p-4 hover:border-white/10 transition-all">
              <div className="flex items-start gap-3">
                <div className="p-2 rounded-ctl bg-[var(--module-accent-soft)] flex-shrink-0">
                  <Plug className="w-4 h-4 text-[var(--module-accent)]" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-body font-bold text-slate-200">{s.name}</span>
                    <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-400">
                      {t(`mcp.${TRANSPORT_KEY[s.transport] || s.transport}`) || s.transport}
                    </span>
                    {!s.enabled && (
                      <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-slate-600/30 text-slate-400">{t("mcp.disabled")}</span>
                    )}
                    <div className="flex items-center gap-0.5 ml-auto flex-shrink-0">
                      <button
                        onClick={() => openEdit(s)}
                        className="p-1 rounded text-slate-600 hover:text-[var(--module-accent)] hover:bg-blue-500/10 cursor-pointer transition-all"
                        title={t("mcp.edit")}
                      >
                        <Edit3 className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => handleDelete(s.id)}
                        className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all"
                        title={t("mcp.delete")}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>

                  <p className="text-tiny text-slate-400 mt-1 truncate font-mono">
                    {s.transport === "stdio"
                      ? `${s.command} ${s.args.join(" ")}`.trim()
                      : s.url}
                  </p>

                  <div className="flex items-center gap-2 mt-3 pt-2 border-t border-white/[0.03]">
                    <span className="text-micro text-slate-500 flex-shrink-0">{t("mcp.forProject")}</span>
                    {(() => {
                      const installed = projMcps.find((p) => p.managed && p.name === s.name);
                      const busy = projBusy === s.id;
                      return (
                        <button
                          onClick={() => (installed ? removeFromProject(s.name, true) : installToProject(s.id))}
                          disabled={!selProject || busy}
                          className={`flex items-center gap-1 px-2 py-0.5 rounded-md text-micro font-semibold cursor-pointer transition-all border disabled:opacity-40 disabled:cursor-not-allowed ${
                            installed
                              ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-400"
                              : "bg-slate-800 border-white/5 text-slate-400 hover:text-slate-200 hover:border-white/15"
                          }`}
                        >
                          <span className={`w-1.5 h-1.5 rounded-full ${installed ? "bg-emerald-400" : "bg-slate-600"}`} />
                          {installed ? t("mcp.installedIn", { name: projects.find((p) => p.id === selProject)?.name ?? "" }) : t("mcp.installToProject")}
                        </button>
                      );
                    })()}
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* 项目维度：已装 / 可装。放在仓库列表之后，作为「装给谁」的那一栏 */}
      <div className="p-3 rounded-card bg-white/[0.02] border border-white/10 space-y-2">
        <div className="flex items-center justify-between gap-2">
          <span className="text-body font-bold text-slate-300 flex items-center gap-1.5">
            <FolderOpen className="w-3.5 h-3.5" /> {t("mcp.projectScope")}
          </span>
          {projects.length > 0 && (
            <select
              value={selProject}
              onChange={(e) => setSelProject(e.target.value)}
              className="appearance-none px-2 py-1 rounded-ctl bg-slate-800 border border-white/10 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent-ring)] cursor-pointer max-w-[240px]"
            >
              {projects.map((p) => (
                <option key={p.id} value={p.id}>{p.name || p.path}</option>
              ))}
            </select>
          )}
        </div>

        {projects.length === 0 ? (
          <p className="text-tiny text-slate-500">{t("mcp.noProjects")}</p>
        ) : (
          <>
            {projMcps.length === 0 ? (
              <p className="text-tiny text-slate-500">{t("mcp.projectNone")}</p>
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
            <p className="text-micro text-slate-500">
              {t("mcp.projectFileHint", { name: ".mcp.json" })}
            </p>
          </>
        )}
      </div>

      {/* 一次性清理：全局配置里的 MCP 对所有项目生效，是上下文被污染的根源 */}
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

      {/* Info */}
      <div className="p-3 rounded-card bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] text-tiny text-slate-400 space-y-1">
        <p className="font-semibold text-[var(--module-accent)]">{t("mcp.aboutScope")}</p>
        <p>{t("mcp.aboutScopeDesc")}</p>
        <p className="text-amber-300/90">{t("mcp.aboutTrust")}</p>
      </div>
    </div>
  );
}
