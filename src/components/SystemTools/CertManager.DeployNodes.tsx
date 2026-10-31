import { useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Plus, Trash2, Cable } from "lucide-react";
import { NODE_TYPE_LABELS, fmtDate, type DeployNode } from "./CertManager.model";
import { NodeForm } from "./CertManager.NodeForm";
// ---- 部署节点 ----
export function DeployNodes() {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<DeployNode[]>([]);
  const [editing, setEditing] = useState<DeployNode | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [testMsg, setTestMsg] = useState("");

  const refresh = useCallback(async () => {
    setNodes(await invoke<DeployNode[]>("deploy_node_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const del = async (id: string) => {
    if (!confirm(t("certmgr.delNodeConfirm"))) return;
    await invoke("deploy_node_delete", { id });
    refresh();
  };
  const test = async (id: string) => {
    setTestMsg(t("certmgr.testing"));
    try {
      const r = await invoke<string>("deploy_node_test", { id });
      setTestMsg(r);
    } catch (e) {
      setTestMsg(t("certmgr.testFail", { err: String(e) }));
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">{t("certmgr.nodesTitle")}</h3>
        <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newNode")}
        </button>
      </div>

      {showForm && <NodeForm initial={editing} onDone={() => { setShowForm(false); setEditing(null); refresh(); }} />}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {nodes.map((n) => (
          <div key={n.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <div className="text-body font-semibold text-slate-200">{n.name}</div>
              <span className="text-tiny px-2 py-0.5 rounded bg-sky-500/20 text-sky-300">{NODE_TYPE_LABELS[n.type] ?? n.type}</span>
            </div>
            <div className="text-tiny text-slate-500 mt-1">{t("certmgr.deployInfo", { days: n.deploy_before_days, time: fmtDate(n.last_deploy_at) })}</div>
            {n.type === "local" && <div className="text-tiny text-slate-400 mt-1 break-all">本机目录：{n.config.output_dir || "未配置"}</div>}
            {n.last_deploy_error && <div className="text-tiny text-rose-400 mt-1">⚠ {n.last_deploy_error}</div>}
            <div className="flex gap-3 mt-2 text-caption">
              <button onClick={() => { setEditing(n); setShowForm(true); }} className="text-slate-300 hover:text-white">{t("certmgr.edit")}</button>
              <button onClick={() => test(n.id)} className="text-sky-400 hover:text-sky-300 flex items-center gap-1"><Cable className="w-3 h-3" />{t("certmgr.test")}</button>
              <button onClick={() => del(n.id)} className="text-rose-400 hover:text-rose-300 flex items-center gap-1"><Trash2 className="w-3 h-3" />{t("certmgr.delete")}</button>
            </div>
          </div>
        ))}
        {nodes.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.nodesEmpty")}</div>}
      </div>
      {testMsg && <div className="text-caption text-slate-400">{t("certmgr.testLabel", { msg: testMsg })}</div>}
    </div>
  );
}
