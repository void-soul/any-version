// 覆写页 —— 1:1 复刻 clash-party src/renderer/src/pages/override.tsx
// （URL 导入(剪贴板粘贴) / 打开本地文件 / 新建 YAML·JS / 卡片：远程更新+编辑信息+
//   编辑文件+执行日志(js)+删除 / 双击编辑文件 / 排序(上下移替代拖拽) / global 标识）
import { useEffect, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import { RefreshCw, MoreVertical, Plus, ClipboardPaste, ArrowUp, ArrowDown } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import { cardCls, Modal, btnSec, btnPrimary, inputCls, labelCls, tagCls, Toggle } from "./ui";

const DEFAULT_YAML = "# https://clashparty.org/docs/guide/override/yaml\n";
const DEFAULT_JS = "// https://clashparty.org/docs/guide/override/javascript\nfunction main(config) {\n  return config\n}";

function fromNow(ts?: number): string {
  if (!ts) return "";
  const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

export default function OverridesPanel() {
  const [items, setItems] = useState<any[]>([]);
  const [url, setUrl] = useState("");
  const [importing, setImporting] = useState(false);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [addMenu, setAddMenu] = useState(false);
  const [editInfo, setEditInfo] = useState<any>(null);
  const [editFile, setEditFile] = useState<any>(null);
  const [execLog, setExecLog] = useState<any>(null);
  const [updating, setUpdating] = useState<Record<string, boolean>>({});
  const [msg, setMsg] = useState("");
  const fileRef = useRef<HTMLInputElement>(null);

  const refresh = async () => {
    try { setItems(((await mihomoApi.getOverrideConfig())?.items) || []); } catch {}
  };
  useEffect(() => { refresh(); }, []);

  const genId = () => "o" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);

  const addItem = async (item: any, content?: string) => {
    await mihomoApi.addOverride(item);
    if (content !== undefined) await mihomoApi.setOverride(item.id, content);
    await refresh();
  };

  // 复刻 handleImport：URL 导入 remote，扩展名按 .js 判断
  const handleImport = async () => {
    setImporting(true);
    setMsg("");
    try {
      const u = new URL(url);
      const name = decodeURIComponent(u.pathname.split("/").pop() || "远程覆写");
      const id = genId();
      const item = {
        id, name, type: "remote", url,
        ext: u.pathname.endsWith(".js") ? "js" : "yaml",
        global: false, updated: Math.floor(Date.now() / 1000),
      };
      const resp = await fetch(url);
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      const content = await resp.text();
      await addItem(item, content);
      setUrl("");
    } catch (e: any) {
      setMsg(`导入失败: ${e}`);
    } finally { setImporting(false); }
  };

  // 复刻 remote 刷新：重新下载 + 热重载
  const updateRemote = async (item: any) => {
    if (!item.url) return;
    setUpdating((s) => ({ ...s, [item.id]: true }));
    try {
      const resp = await fetch(item.url);
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      await mihomoApi.setOverride(item.id, await resp.text());
      await mihomoApi.updateOverride({ ...item, updated: Math.floor(Date.now() / 1000) });
      await mihomoApi.updateRuntimeConfig();
      await refresh();
    } catch (e: any) { setMsg(`更新失败: ${e}`); }
    finally { setUpdating((s) => ({ ...s, [item.id]: false })); }
  };

  // 打开本地文件（复刻 actions.open）
  const onLocalFile = async (f: File) => {
    if (!f.name.endsWith(".js") && !f.name.endsWith(".yaml")) { setMsg("仅支持 .js / .yaml 文件"); return; }
    const content = await f.text();
    await addItem({
      id: genId(), name: f.name, type: "local",
      ext: f.name.endsWith(".js") ? "js" : "yaml", global: false,
    }, content);
  };

  const move = async (index: number, dir: -1 | 1) => {
    const next = [...items];
    const ni = index + dir;
    if (ni < 0 || ni >= next.length) return;
    [next[index], next[ni]] = [next[ni], next[index]];
    setItems(next);
    await mihomoApi.setOverrideConfig({ items: next });
  };

  const remove = async (id: string) => {
    await mihomoApi.removeOverride(id);
    await mihomoApi.updateRuntimeConfig();
    refresh();
  };

  return (
    <div className="space-y-3" onClick={() => { setMenuFor(null); setAddMenu(false); }}>
      {/* 顶栏：URL 导入 + 新建（复刻 sticky 头部） */}
      <div className="flex gap-2 items-center">
        <div className="relative flex-1">
          <input
            className={`${inputCls} !pr-9`}
            placeholder="覆写文件 URL"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
          <button
            className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-white cursor-pointer"
            title="粘贴"
            onClick={() => navigator.clipboard.readText().then(setUrl).catch(() => {})}
          >
            <ClipboardPaste className="w-3.5 h-3.5" />
          </button>
        </div>
        <button className={btnPrimary} disabled={!url || importing} onClick={handleImport}>
          {importing ? "导入中..." : "导入"}
        </button>
        <div className="relative">
          <button className={btnPrimary} onClick={(e) => { e.stopPropagation(); setAddMenu((p) => !p); }}>
            <Plus className="w-3.5 h-3.5" />
          </button>
          {addMenu && (
            <div className="absolute right-0 top-9 z-30 w-36 rounded-xl border border-white/10 bg-[#1a1f2d] shadow-xl overflow-hidden"
              onClick={(e) => e.stopPropagation()}>
              {[
                ["打开文件", () => fileRef.current?.click()],
                ["新建 YAML", () => addItem({ id: genId(), name: "新建 YAML", type: "local", ext: "yaml", global: false }, DEFAULT_YAML)],
                ["新建 JS", () => addItem({ id: genId(), name: "新建 JS", type: "local", ext: "js", global: false }, DEFAULT_JS)],
              ].map(([t, fn]: any) => (
                <button key={t} className="w-full text-left px-3 py-2 text-[11px] text-slate-200 hover:bg-white/10 cursor-pointer"
                  onClick={() => { fn(); setAddMenu(false); }}>
                  {t}
                </button>
              ))}
            </div>
          )}
        </div>
        <input ref={fileRef} type="file" accept=".js,.yaml" className="hidden"
          onChange={(e) => { const f = e.target.files?.[0]; if (f) onLocalFile(f); e.target.value = ""; }} />
      </div>
      {msg && <div className="text-[11px] text-rose-300 px-1">{msg}</div>}

      {/* 卡片网格 */}
      <div className="grid grid-cols-2 lg:grid-cols-3 gap-2">
        {items.map((item, index) => (
          <div
            key={item.id}
            className={`${cardCls} p-3 cursor-pointer hover:border-white/20 relative`}
            onDoubleClick={() => setEditFile(item)}
            onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); setMenuFor(item.id); }}
          >
            <div className="flex items-center justify-between gap-1">
              <h3 className="text-[13px] font-bold text-white truncate" title={item.name}>{item.name}</h3>
              <div className="flex items-center flex-shrink-0">
                <button className="p-1 rounded-md hover:bg-white/10 text-slate-400 cursor-pointer" title="上移"
                  onClick={(e) => { e.stopPropagation(); move(index, -1); }}>
                  <ArrowUp className="w-3 h-3" />
                </button>
                <button className="p-1 rounded-md hover:bg-white/10 text-slate-400 cursor-pointer" title="下移"
                  onClick={(e) => { e.stopPropagation(); move(index, 1); }}>
                  <ArrowDown className="w-3 h-3" />
                </button>
                {item.type === "remote" && (
                  <button className="p-1 rounded-md hover:bg-white/10 text-slate-300 cursor-pointer" title="更新"
                    onClick={(e) => { e.stopPropagation(); updateRemote(item); }}>
                    <RefreshCw className={`w-3.5 h-3.5 ${updating[item.id] ? "animate-spin" : ""}`} />
                  </button>
                )}
                <button className="p-1 rounded-md hover:bg-white/10 text-slate-300 cursor-pointer"
                  onClick={(e) => { e.stopPropagation(); setMenuFor(menuFor === item.id ? null : item.id); }}>
                  <MoreVertical className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
            <div className="mt-2 flex items-center justify-between">
              <div className="flex gap-1.5">
                {item.global && <span className="text-[10px] px-2 py-0.5 rounded-md bg-emerald-500/15 text-emerald-300 border border-emerald-500/25">全局</span>}
                <span className={tagCls}>{item.ext === "yaml" ? "YAML" : "JavaScript"}</span>
              </div>
              {item.type === "remote" && <span className="text-[10px] text-slate-500">{fromNow(item.updated)}</span>}
            </div>

            {/* 菜单（复刻 menuItems：编辑信息/编辑文件/执行日志(js)/删除） */}
            {menuFor === item.id && (
              <div className="absolute right-2 top-9 z-30 w-32 rounded-xl border border-white/10 bg-[#1a1f2d] shadow-xl overflow-hidden"
                onClick={(e) => e.stopPropagation()}>
                <button className="w-full text-left px-3 py-2 text-[11px] text-slate-200 hover:bg-white/10 cursor-pointer"
                  onClick={() => { setEditInfo(item); setMenuFor(null); }}>编辑信息</button>
                <button className="w-full text-left px-3 py-2 text-[11px] text-slate-200 hover:bg-white/10 cursor-pointer"
                  onClick={() => { setEditFile(item); setMenuFor(null); }}>编辑文件</button>
                {item.ext === "js" && (
                  <button className="w-full text-left px-3 py-2 text-[11px] text-slate-200 hover:bg-white/10 cursor-pointer border-b border-white/5"
                    onClick={() => { setExecLog(item); setMenuFor(null); }}>执行日志</button>
                )}
                <button className="w-full text-left px-3 py-2 text-[11px] text-rose-300 hover:bg-rose-500/15 cursor-pointer"
                  onClick={() => { remove(item.id); setMenuFor(null); }}>删除</button>
              </div>
            )}
          </div>
        ))}
        {items.length === 0 && (
          <div className={`${cardCls} p-6 text-center text-xs text-slate-400 col-span-full`}>
            暂无覆写文件，可通过 URL 导入或新建
          </div>
        )}
      </div>

      {editInfo && <EditOverrideInfoModal item={editInfo} onClose={() => setEditInfo(null)} onSaved={refresh} />}
      {editFile && <EditOverrideFileModal item={editFile} onClose={() => setEditFile(null)} />}
      {execLog && <ExecLogModal item={execLog} onClose={() => setExecLog(null)} />}
    </div>
  );
}

// 复刻 edit-info-modal：名称 / URL(remote) / 全局开关
function EditOverrideInfoModal({ item, onClose, onSaved }: any) {
  const [name, setName] = useState(item.name || "");
  const [u, setU] = useState(item.url || "");
  const [global, setGlobal] = useState(!!item.global);
  const [saving, setSaving] = useState(false);
  const save = async () => {
    setSaving(true);
    try {
      await mihomoApi.updateOverride({ ...item, name, url: item.type === "remote" ? u : item.url, global });
      await mihomoApi.updateRuntimeConfig();
      onSaved();
      onClose();
    } catch (e: any) {
      alert(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  };
  return (
    <Modal title="编辑信息" onClose={onClose} busy={saving} busyText="保存并重载配置…"
      footer={
        <>
          <button className={btnSec} disabled={saving} onClick={onClose}>取消</button>
          <button className={btnPrimary} disabled={saving} onClick={save}>保存</button>
        </>
      }>
      <div>
        <label className={labelCls}>名称</label>
        <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      {item.type === "remote" && (
        <div>
          <label className={labelCls}>URL</label>
          <input className={inputCls} value={u} onChange={(e) => setU(e.target.value)} />
        </div>
      )}
      <Toggle label="作用于所有订阅（全局）" v={global} onChange={setGlobal} />
    </Modal>
  );
}

// 复刻 edit-file-modal：编辑内容，保存后热重载
function EditOverrideFileModal({ item, onClose }: any) {
  const [content, setContent] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    mihomoApi.getOverride(item.id).then((s: string) => { setContent(s); setLoaded(true); }).catch(() => setLoaded(true));
  }, [item.id]);
  const save = async () => {
    setSaving(true);
    try {
      await mihomoApi.setOverride(item.id, content);
      await mihomoApi.updateRuntimeConfig();
      onClose();
    } catch (e: any) {
      alert(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  };
  return (
    <Modal title={`编辑文件 - ${item.name}`} wide onClose={onClose} busy={saving} busyText="保存并重载配置…"
      footer={
        <>
          <button className={btnSec} disabled={saving} onClick={onClose}>取消</button>
          <button className={btnPrimary} disabled={saving} onClick={save}>保存</button>
        </>
      }>
      {loaded ? (
        <div className="w-full h-96 rounded-xl border border-white/10 overflow-hidden">
          <Editor
            height="100%"
            language={item?.ext === "js" ? "javascript" : "yaml"}
            theme="vs-dark"
            value={content}
            onChange={(v) => setContent(v ?? "")}
            options={{ minimap: { enabled: false }, fontSize: 12, scrollBeyondLastLine: false, wordWrap: "on", automaticLayout: true }}
          />
        </div>
      ) : <div className="text-xs text-slate-400 p-4">加载中...</div>}
    </Modal>
  );
}

// 复刻 exec-log-modal：显示 JS 覆写执行日志
function ExecLogModal({ item, onClose }: any) {
  const [log, setLog] = useState<string[]>([]);
  useEffect(() => {
    mihomoApi.getOverrideExecLog(item.id).then((l: string[]) => setLog(l || [])).catch(() => setLog([]));
  }, [item.id]);
  return (
    <Modal title={`执行日志 - ${item.name}`} wide onClose={onClose}
      footer={<button className={btnSec} onClick={onClose}>关闭</button>}>
      <div className="max-h-96 overflow-y-auto font-mono text-[11px] text-slate-300 space-y-1">
        {log.length === 0 && <div className="text-slate-500">暂无日志（配置生成时执行 JS 覆写会记录输出）</div>}
        {log.map((l, i) => <div key={i} className="break-all select-text">{l}</div>)}
      </div>
    </Modal>
  );
}
