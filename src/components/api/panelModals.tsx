import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Plus, Trash2, X, Link2, Database, FlaskConical, Folder, ListTree, Braces,
} from "lucide-react";
import type { ApiProject, ApiEnvironment, ApiModule, PresetHeaderSet, KeyValueItem } from "./types";
import { KvEditor, ACCENT } from "./panelParts";

// ─── 预设 Headers 弹窗 ───
export function PresetHeadersModal({ projectId, sets, onClose, onChanged }: {
  projectId: string;
  sets: PresetHeaderSet[];
  onClose: () => void;
  onChanged: () => void;
}) {
  const [local, setLocal] = useState<PresetHeaderSet[]>(sets);
  const update = (i: number, patch: Partial<PresetHeaderSet>) => {
    setLocal(local.map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  };
  const add = () => {
    setLocal([...local, { id: "", project_id: projectId, name: `预设 ${local.length + 1}`, headers: [], created_at: "" }]);
  };
  const saveAll = async () => {
    for (const s of local) {
      await invoke("api_save_preset_headers", { set: s });
    }
    onChanged();
    onClose();
  };
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[620px] max-h-[80vh] overflow-hidden glass-panel rounded-2xl border border-white/10 shadow-2xl flex flex-col" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Link2 className="w-4 h-4" style={{ color: ACCENT }} /> 预设 Headers（项目级）
          </div>
          <button onClick={onClose} className="p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="flex-1 overflow-y-auto p-4 space-y-3">
          {local.map((s, i) => (
            <div key={i} className="rounded-xl border border-white/10 bg-black/20 p-3 space-y-2">
              <div className="flex items-center gap-2">
                <input value={s.name} onChange={(e) => update(i, { name: e.target.value })} className="flex-1 bg-transparent border border-white/10 rounded-md px-2 py-1 text-xs font-semibold text-slate-100 focus:outline-none" />
                <button onClick={() => setLocal(local.filter((_, idx) => idx !== i))} className="p-1 text-slate-500 hover:text-rose-400 cursor-pointer"><Trash2 className="w-3.5 h-3.5" /></button>
              </div>
              <KvEditor items={s.headers} onChange={(h) => update(i, { headers: h })} placeholderKey="Header 名" placeholderValue="值" withDescription={false} />
            </div>
          ))}
          <button onClick={add} className="flex items-center gap-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
            <Plus className="w-3.5 h-3.5" /> 新建预设集合
          </button>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button onClick={saveAll} className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer" style={{ background: ACCENT }}>保存</button>
        </div>
      </div>
    </div>
  );
}

// ─── 变量集合弹窗（矩阵编辑：每列一个环境，每行一个变量名）───
export function EnvModal({ projectId, envs, activeEnvId, onClose, onChanged }: {
  projectId: string;
  envs: ApiEnvironment[];
  activeEnvId: string | null;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [local, setLocal] = useState<ApiEnvironment[]>(envs);
  const [active, setActive] = useState<string | null>(activeEnvId);

  const updateEnv = (i: number, patch: Partial<ApiEnvironment>) => {
    setLocal(prev => prev.map((e, idx) => (idx === i ? { ...e, ...patch } : e)));
  };

  const addEnv = async () => {
    const created = await invoke<ApiEnvironment>("api_create_environment", {
      projectId, name: `环境${local.length + 1}`, variables: {},
    });
    setLocal(prev => [...prev, created]);
    if (!active) setActive(created.id);
  };

  const saveAll = async () => {
    for (const e of local) {
      await invoke("api_update_environment", { env: e });
    }
    if (active && active !== activeEnvId) {
      await invoke("api_set_active_env", { projectId, envId: active });
    }
    onChanged();
    onClose();
  };

  // 变量名并集（保持首次出现的顺序），保证各环境列对齐
  const allKeys = (() => {
    const seen: string[] = [];
    for (const e of local) for (const k of Object.keys(e.variables)) if (!seen.includes(k)) seen.push(k);
    return seen;
  })();

  const setVar = (envIdx: number, key: string, value: string) => {
    setLocal(prev => prev.map((e, i) => (i === envIdx ? { ...e, variables: { ...e.variables, [key]: value } } : e)));
  };

  const renameVar = (oldKey: string, newKey: string) => {
    if (!newKey.trim() || newKey === oldKey) return;
    setLocal(prev => prev.map(e => {
      const vars = { ...e.variables };
      if (oldKey in vars) { vars[newKey] = vars[oldKey]; delete vars[oldKey]; }
      return { ...e, variables: vars };
    }));
  };

  const addRow = () => {
    const key = `变量${allKeys.length + 1}`;
    setLocal(prev => prev.map(e => ({ ...e, variables: { ...e.variables, [key]: "" } })));
  };

  const deleteRow = (key: string) => {
    setLocal(prev => prev.map(e => {
      const vars = { ...e.variables }; delete vars[key]; return { ...e, variables: vars };
    }));
  };

  const cellCls = "bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[860px] max-w-[95vw] max-h-[82vh] overflow-hidden glass-panel rounded-2xl border border-white/10 shadow-2xl flex flex-col" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Database className="w-4 h-4" style={{ color: ACCENT }} /> 变量集合（环境）
          </div>
          <div className="flex items-center gap-2">
            <select
              value={active ?? ""}
              onChange={(e) => setActive(e.target.value)}
              className="bg-black/30 border border-white/10 rounded-md px-2 py-1 text-[11px] text-slate-200 focus:outline-none"
              title="当前生效的环境（请求变量取自该列）"
            >
              {local.map(e => <option key={e.id} value={e.id}>{e.name}</option>)}
            </select>
            <button onClick={onClose} className="p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
          </div>
        </div>
        <div className="flex-1 overflow-auto p-4">
          {local.length === 0 ? (
            <div className="py-10 text-center text-xs text-slate-500">暂无环境，点击下方“新建环境”创建第一列</div>
          ) : (
            <table className="w-full border-separate border-spacing-0 text-xs">
              <thead>
                <tr>
                  <th className="sticky left-0 z-10 bg-[#0d1524] px-2 py-1.5 text-left text-[10px] font-semibold text-slate-400 border-b border-white/10">变量名</th>
                  {local.map((e, i) => (
                    <th key={e.id} className={`px-1.5 py-1 border-b border-white/10 ${e.id === active ? "bg-[color-mix(in_srgb,var(--module-accent)_10%,transparent)]" : "bg-black/20"}`}>
                      <div className="flex items-center gap-1">
                        {e.id === active && <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--module-accent)" }} title="当前生效" />}
                        <input
                          value={e.name}
                          onChange={(ev) => updateEnv(i, { name: ev.target.value })}
                          className={`w-full min-w-[110px] bg-transparent border ${e.id === active ? "border-[var(--module-accent)]/50" : "border-white/10"} rounded-md px-1.5 py-0.5 text-[11px] font-semibold text-slate-100 focus:outline-none`}
                        />
                        <button
                          onClick={() => setActive(e.id)}
                          className="shrink-0 p-0.5 text-[9px] text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                          title="设为当前环境"
                        >当前</button>
                        <button
                          onClick={async () => {
                            await invoke("api_delete_environment", { envId: e.id });
                            setLocal(prev => prev.filter((_, idx) => idx !== i));
                            if (active === e.id) setActive(local.find(x => x.id !== e.id)?.id ?? null);
                          }}
                          className="shrink-0 p-0.5 text-slate-500 hover:text-rose-400 cursor-pointer"
                          title="删除此环境列"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {allKeys.map((key) => (
                  <tr key={key} className="group">
                    <td className="sticky left-0 z-10 bg-[#0d1524] px-2 py-1 border-b border-white/5">
                      <div className="flex items-center gap-1">
                        <input
                          defaultValue={key}
                          onBlur={(e) => renameVar(key, e.target.value.trim())}
                          className={`w-full min-w-[110px] bg-transparent border border-transparent rounded-md px-1 py-0.5 text-[11px] font-medium text-slate-200 focus:border-[var(--module-accent)]/50 focus:outline-none`}
                          title="编辑变量名（失焦后同步到所有环境）"
                        />
                        <button
                          onClick={() => deleteRow(key)}
                          className="shrink-0 p-0.5 text-slate-600 opacity-0 group-hover:opacity-100 hover:text-rose-400 cursor-pointer"
                          title="删除此行（所有环境）"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </td>
                    {local.map((e, ei) => (
                      <td key={e.id} className={`px-1.5 py-1 border-b border-white/5 ${e.id === active ? "bg-[color-mix(in_srgb,var(--module-accent)_4%,transparent)]" : ""}`}>
                        <input
                          value={String(e.variables[key] ?? "")}
                          onChange={(ev) => setVar(ei, key, ev.target.value)}
                          placeholder="{{$guid}} 等随机变量"
                          className={`${cellCls} w-full min-w-[120px]`}
                        />
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <div className="mt-3 flex items-center gap-2">
            <button onClick={addRow} className="flex items-center gap-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
              <Plus className="w-3.5 h-3.5" /> 添加变量行
            </button>
            <button onClick={addEnv} className="flex items-center gap-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
              <Plus className="w-3.5 h-3.5" /> 新建环境列
            </button>
          </div>
          <div className="mt-2 text-[10px] text-slate-500 space-y-0.5">
            提示：每组环境代表一列，每行一个变量名；不同环境的变量名自动对齐，只改值即可。
            请求中通过 <code className="text-[var(--module-accent)]">{"{{变量名}}"}</code> 引用当前生效环境的值。
          </div>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button onClick={saveAll} className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer" style={{ background: ACCENT }}>
            保存
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 项目新建/编辑弹窗 ───
export function ProjectModal({ project, onClose, onSave }: {
  project: ApiProject | null;
  onClose: () => void;
  onSave: (name: string, description: string, commonHeaders: KeyValueItem[], commonParams: KeyValueItem[], commonBody: KeyValueItem[]) => void;
}) {
  const [name, setName] = useState(project?.name ?? "");
  const [description, setDescription] = useState(project?.description ?? "");
  const [commonHeaders, setCommonHeaders] = useState<KeyValueItem[]>(project?.common_headers ?? []);
  const [commonParams, setCommonParams] = useState<KeyValueItem[]>(project?.common_params ?? []);
  const [commonBody, setCommonBody] = useState<KeyValueItem[]>(project?.common_body ?? []);
  const editing = !!project;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-[560px] glass-panel rounded-2xl border border-white/10 shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
          <FlaskConical className="w-4 h-4" style={{ color: ACCENT }} />
          <span className="text-sm font-semibold text-white">{editing ? "编辑项目" : "新建项目"}</span>
          <button onClick={onClose} className="ml-auto p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-4 space-y-3">
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">项目名称</span>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如：电商后台、开放平台…"
              onKeyDown={(e) => e.key === "Enter" && name.trim() && onSave(name.trim(), description, commonHeaders, commonParams, commonBody)}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-100 focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">简介</span>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="这个项目面向什么场景？包含哪些模块？…"
              rows={2}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 resize-none focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
          <div>
            <div className="flex items-center gap-1.5 mb-1">
              <Link2 className="w-3 h-3" style={{ color: ACCENT }} />
              <span className="text-[11px] text-slate-400">通用 Headers（接口模板）</span>
              <span className="text-[9px] text-slate-600">新建接口时自动附加</span>
            </div>
            <KvEditor items={commonHeaders} onChange={setCommonHeaders} placeholderKey="Header 名" placeholderValue="值" withDescription={false} />
          </div>
          <div>
            <div className="flex items-center gap-1.5 mb-1">
              <ListTree className="w-3 h-3" style={{ color: ACCENT }} />
              <span className="text-[11px] text-slate-400">通用 Params（接口模板）</span>
              <span className="text-[9px] text-slate-600">新建接口时自动附加</span>
            </div>
            <KvEditor items={commonParams} onChange={setCommonParams} placeholderKey="参数名" placeholderValue="值" withDescription={false} />
          </div>
          <div>
            <div className="flex items-center gap-1.5 mb-1">
              <Braces className="w-3 h-3" style={{ color: ACCENT }} />
              <span className="text-[11px] text-slate-400">通用 Body 参数（接口模板）</span>
              <span className="text-[9px] text-slate-600">新建接口时自动附加到 x-www-form-urlencoded 与 form-data</span>
            </div>
            <KvEditor items={commonBody} onChange={setCommonBody} placeholderKey="参数名" placeholderValue="值" withDescription={false} />
          </div>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button
            onClick={() => name.trim() && onSave(name.trim(), description, commonHeaders, commonParams, commonBody)}
            disabled={!name.trim()}
            className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer disabled:opacity-50"
            style={{ background: ACCENT }}
          >
            {editing ? "保存" : "创建"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 模块新建/编辑弹窗 ───
export function ModuleModal({ module, onClose, onSave }: {
  module: ApiModule | null;
  onClose: () => void;
  onSave: (name: string, description: string) => void;
}) {
  const [name, setName] = useState(module?.name ?? "");
  const [description, setDescription] = useState(module?.description ?? "");
  const editing = !!module;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-[440px] glass-panel rounded-2xl border border-white/10 shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
          <Folder className="w-4 h-4" style={{ color: ACCENT }} />
          <span className="text-sm font-semibold text-white">{editing ? "编辑模块" : "新建模块"}</span>
          <button onClick={onClose} className="ml-auto p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="p-4 space-y-3">
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">模块名称</span>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如：订单、用户、商品…"
              onKeyDown={(e) => e.key === "Enter" && name.trim() && onSave(name.trim(), description)}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-100 focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">简介</span>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="这个模块包含哪些接口？用途说明…"
              rows={3}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 resize-none focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button
            onClick={() => name.trim() && onSave(name.trim(), description)}
            disabled={!name.trim()}
            className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer disabled:opacity-50"
            style={{ background: ACCENT }}
          >
            {editing ? "保存" : "创建"}
          </button>
        </div>
      </div>
    </div>
  );
}