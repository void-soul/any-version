import { useState, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { NODE_FIELDS, NODE_TYPE_LABELS, type DeployNode, type FieldSpec } from "./CertManager.model";
export function NodeForm({ initial, onDone }: { initial: DeployNode | null; onDone: () => void }) {
  const { t } = useTranslation();
  const makeData = (cfg: Record<string, string>, fields: FieldSpec[]) => {
    const d: Record<string, string> = {};
    for (const f of fields) d[f.key] = cfg[f.key] ?? "";
    return d;
  };
  // `type` 是 Rust 侧序列化出来的名字（cert.rs 里 node_type 带 rename = "type"）。
  // 这里不能再写 node_type —— 那样恒为 undefined，任何非 linux 节点都会退化成 linux，
  // 保存时还会把这个错误类型写回去。
  const initialType = initial?.type || "windows";
  const [nodeType, setNodeType] = useState(initialType);
  const [name, setName] = useState(initial?.name || "");
  const [data, setData] = useState<Record<string, string>>(() =>
    makeData(initial?.config ?? {}, NODE_FIELDS[initialType] || [])
  );
  const [deployBefore, setDeployBefore] = useState(initial?.deploy_before_days || 7);
  const [note, setNote] = useState(initial?.note || "");
  const [err, setErr] = useState("");
  // 首次挂载保留 initial 配置；只有用户切换类型时才清空字段
  const firstTypeRun = useRef(true);

  useEffect(() => {
    if (firstTypeRun.current) {
      firstTypeRun.current = false;
      return;
    }
    setData(makeData({}, NODE_FIELDS[nodeType] || []));
  }, [nodeType]);

  const setField = (key: string, val: string) => {
    setData((prev) => ({ ...prev, [key]: val }));
  };

  const submit = async () => {
    setErr("");
    try {
      await invoke("deploy_node_upsert", {
        id: initial?.id,
        nodeType,
        name,
        config: data,
        deployBeforeDays: deployBefore,
        note,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  const fields = NODE_FIELDS[nodeType] || [];

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.type")}
          <select value={nodeType} onChange={(e) => setNodeType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {Object.keys(NODE_FIELDS).map((k) => (
              <option key={k} value={k} className="bg-slate-800 text-slate-200">{NODE_TYPE_LABELS[k] ?? k}</option>
            ))}
          </select>
        </label>
      </div>
      <div className="text-caption text-slate-400 font-semibold">{t("certmgr.configItems")}</div>
      <div className="grid grid-cols-2 gap-3">
        {fields.map((f) => (
          <label key={f.key} className={`text-caption text-slate-400 ${f.textarea ? "col-span-2" : ""}`}>
            {t(f.label)}
            {f.select ? (
              <select
                value={data[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none"
              >
                <option value="">{t("certmgr.select")}</option>
                {f.select.map((o) => (
                  <option key={o.value} value={o.value} className="bg-slate-800 text-slate-200">{o.label}</option>
                ))}
              </select>
            ) : f.textarea ? (
              <textarea
                rows={2}
                value={data[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none font-mono text-tiny"
                placeholder={f.placeholder || ""}
              />
            ) : (
              <input
                type={f.type}
                value={data[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none"
                placeholder={f.placeholder || ""}
              />
            )}
          </label>
        ))}
      </div>
      {nodeType === "windows" && (
        <p className="ui-note ui-note-info">
          {t("certmgr.receiverHint")}
        </p>
      )}
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.deployDays")}
          <input type="number" value={deployBefore} onChange={(e) => setDeployBefore(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.remark")}
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}
