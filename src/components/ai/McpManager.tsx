import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
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
} from "lucide-react";

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

interface McpToolInfo {
  id: string;
  label: string;
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

const TRANSPORT_LABEL: Record<string, string> = {
  stdio: "本地 (stdio)",
  http: "HTTP",
  sse: "SSE",
};

export default function McpManager() {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [mcpTools, setMcpTools] = useState<McpToolInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [togglingMap, setTogglingMap] = useState<Record<string, boolean>>({});

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

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [srv, tools] = await Promise.all([
        invoke<McpServer[]>("get_mcp_servers"),
        invoke<McpToolInfo[]>("get_mcp_tools"),
      ]);
      setServers(srv);
      setMcpTools(tools);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

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
    if (!fName.trim()) { alert("请填写服务器名称"); return; }
    if (fTransport === "stdio") {
      if (!fCommand.trim()) { alert("stdio 类型必须填写启动命令"); return; }
    } else if (!fUrl.trim()) {
      alert("http/sse 类型必须填写 URL"); return;
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
      alert(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm("确定删除该 MCP 服务器？将从已部署工具的配置中移除。")) return;
    try {
      await invoke("delete_mcp_server", { id });
      await load();
    } catch (e: any) { alert(`删除失败: ${e}`); }
  };

  const handleToggle = async (id: string, toolId: string, current: boolean) => {
    const key = `${id}:${toolId}`;
    setTogglingMap((p) => ({ ...p, [key]: true }));
    try {
      await invoke("toggle_mcp_tool", { id, toolId, enabled: !current });
      await load();
    } catch (e: any) { alert(`操作失败: ${e}`); }
    finally { setTogglingMap((p) => ({ ...p, [key]: false })); }
  };

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-slate-500">
        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
        <span className="text-xs">加载中...</span>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-6 space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">MCP 管理</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">管理 Model Context Protocol 服务器，一键部署到各工具</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={load}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <RefreshCw className="w-3 h-3" /> 刷新
          </button>
          <button
            onClick={openAdd}
            className="px-3 py-1.5 rounded-lg bg-violet-600 hover:bg-violet-500 text-white text-[10px] font-semibold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-violet-500/10"
          >
            <Plus className="w-3 h-3" /> 添加服务器
          </button>
        </div>
      </div>

      {/* 添加 / 编辑表单 */}
      {showForm && (
        <div className="p-3 rounded-xl bg-slate-900/40 border border-violet-500/20 space-y-3">
          <div className="flex items-center justify-between">
            <h4 className="text-xs font-bold text-violet-300 flex items-center gap-1.5">
              <Plug className="w-3.5 h-3.5" /> {editingId ? "编辑服务器" : "添加 MCP 服务器"}
            </h4>
            <button onClick={() => { setShowForm(false); resetForm(); }} className="text-slate-500 hover:text-slate-300 cursor-pointer">
              <X className="w-4 h-4" />
            </button>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">名称（工具配置中的唯一键）</label>
              <input
                value={fName}
                onChange={(e) => setFName(e.target.value)}
                placeholder="如 github / context7"
                className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500"
              />
            </div>
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">传输类型</label>
              <select
                value={fTransport}
                onChange={(e) => setFTransport(e.target.value)}
                className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-violet-500"
              >
                <option value="stdio">本地 (stdio)</option>
                <option value="http">HTTP</option>
                <option value="sse">SSE</option>
              </select>
            </div>
          </div>

          {fTransport === "stdio" ? (
            <>
              <div>
                <label className="text-[9px] text-slate-500 block mb-1">启动命令</label>
                <input
                  value={fCommand}
                  onChange={(e) => setFCommand(e.target.value)}
                  placeholder="如 npx"
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500"
                />
              </div>
              <div>
                <label className="text-[9px] text-slate-500 block mb-1">参数（每行一个）</label>
                <textarea
                  value={fArgs}
                  onChange={(e) => setFArgs(e.target.value)}
                  rows={2}
                  placeholder={"-y\n@modelcontextprotocol/server-everything"}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500 resize-none"
                />
              </div>
              <div>
                <label className="text-[9px] text-slate-500 block mb-1">工作目录（可选）</label>
                <input
                  value={fCwd}
                  onChange={(e) => setFCwd(e.target.value)}
                  placeholder="如 ./mcp-servers/python"
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500"
                />
              </div>
              <div>
                <label className="text-[9px] text-slate-500 block mb-1">环境变量（KEY=VALUE，每行一个）</label>
                <textarea
                  value={fEnv}
                  onChange={(e) => setFEnv(e.target.value)}
                  rows={2}
                  placeholder={"API_KEY=xxx"}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500 resize-none"
                />
              </div>
            </>
          ) : (
            <>
              <div>
                <label className="text-[9px] text-slate-500 block mb-1">URL</label>
                <input
                  value={fUrl}
                  onChange={(e) => setFUrl(e.target.value)}
                  placeholder="如 https://mcp.context7.com/mcp"
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500"
                />
              </div>
              <div>
                <label className="text-[9px] text-slate-500 block mb-1">请求头（KEY=VALUE，每行一个）</label>
                <textarea
                  value={fHeaders}
                  onChange={(e) => setFHeaders(e.target.value)}
                  rows={2}
                  placeholder={"Authorization=Bearer xxx"}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500 resize-none"
                />
              </div>
            </>
          )}

          <div>
            <label className="text-[9px] text-slate-500 block mb-1">描述（可选）</label>
            <input
              value={fDescription}
              onChange={(e) => setFDescription(e.target.value)}
              placeholder="如 搜索文档"
              className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-violet-500"
            />
          </div>

          <label className="flex items-center gap-2 text-[10px] text-slate-400 cursor-pointer">
            <input type="checkbox" checked={fEnabled} onChange={(e) => setFEnabled(e.target.checked)} className="accent-violet-500" />
            全局启用（关闭则不部署到任何工具）
          </label>

          <div className="flex justify-end gap-2 pt-1">
            <button
              onClick={() => { setShowForm(false); resetForm(); }}
              className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer"
            >
              取消
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="px-4 py-1.5 rounded-lg bg-violet-600 hover:bg-violet-500 disabled:opacity-40 text-white text-[10px] font-semibold cursor-pointer flex items-center gap-1"
            >
              {saving ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
              {saving ? "保存中..." : "保存并部署"}
            </button>
          </div>
        </div>
      )}

      {/* 已安装服务器列表 */}
      {servers.length === 0 ? (
        <div className="h-48 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-500">
          <Server className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-xs font-bold text-slate-400">暂无 MCP 服务器</span>
          <span className="text-[10px] text-slate-600 mt-1">点击「添加服务器」开始</span>
        </div>
      ) : (
        <div className="space-y-2">
          {servers.map((s) => (
            <div key={s.id} className="rounded-xl bg-slate-900/30 border border-white/5 p-4 hover:border-white/10 transition-all">
              <div className="flex items-start gap-3">
                <div className="p-2 rounded-lg bg-violet-500/10 flex-shrink-0">
                  <Plug className="w-4 h-4 text-violet-400" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-xs font-bold text-slate-200">{s.name}</span>
                    <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-400">
                      {TRANSPORT_LABEL[s.transport] || s.transport}
                    </span>
                    {!s.enabled && (
                      <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-slate-600/30 text-slate-400">已停用</span>
                    )}
                    <div className="flex items-center gap-0.5 ml-auto flex-shrink-0">
                      <button
                        onClick={() => openEdit(s)}
                        className="p-1 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer transition-all"
                        title="编辑"
                      >
                        <Edit3 className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => handleDelete(s.id)}
                        className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all"
                        title="删除"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>

                  <p className="text-[10px] text-slate-400 mt-1 truncate font-mono">
                    {s.transport === "stdio"
                      ? `${s.command} ${s.args.join(" ")}`.trim()
                      : s.url}
                  </p>

                  <div className="flex items-center gap-3 mt-3 pt-2 border-t border-white/[0.03]">
                    <span className="text-[9px] text-slate-500 flex-shrink-0">部署到:</span>
                    <div className="flex flex-wrap gap-1.5">
                      {mcpTools.map((t) => {
                        const enabled = s.enabledTools.includes(t.id);
                        const key = `${s.id}:${t.id}`;
                        const toggling = togglingMap[key] || false;
                        return (
                          <button
                            key={t.id}
                            onClick={() => handleToggle(s.id, t.id, enabled)}
                            disabled={toggling || !s.enabled}
                            className={`flex items-center gap-1 px-2 py-0.5 rounded-md text-[9px] font-semibold cursor-pointer transition-all border disabled:opacity-40 disabled:cursor-not-allowed ${
                              enabled
                                ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-400"
                                : "bg-slate-800 border-white/5 text-slate-600 hover:text-slate-400 hover:border-white/10"
                            }`}
                          >
                            <span className={`w-1.5 h-1.5 rounded-full ${enabled ? "bg-emerald-400" : "bg-slate-600"}`} />
                            {t.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Info */}
      <div className="p-3 rounded-xl bg-violet-500/5 border border-violet-500/10 text-[10px] text-slate-400 space-y-1">
        <p className="font-semibold text-violet-300">关于部署</p>
        <p>保存后会按各工具格式写入其中心配置文件（如 Claude 的 ~/.claude.json、Qwen 的 ~/.qwen/settings.json、OpenCode 的 opencode.json），仅更新 mcpServers / mcp 字段，保留其它内容。重启对应工具即可生效。</p>
      </div>
    </div>
  );
}
