import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Note } from "../shared/Note";
import { SharedModal } from "../shared/Modal";
import { SharedButton } from "../shared/Button";
import { type DockerServiceConfig, type NodeProjectDef } from "./NodeManagerPanel.model";
/// docker 服务的配置弹窗：改对外端口与数据库密码。
///
/// 为什么不做成卡片内联编辑：这两项都要写回配置并**重建容器**才生效，
/// 属于「改错了要收拾」的操作，单独一步确认更稳妥；而且端口被占时
/// 用户需要一个明确的地方去改（否则只能手改数据目录里的文件）。
export function DockerConfigDialog({
  project,
  onClose,
  onSaved,
}: {
  project: NodeProjectDef;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [dir, setDir] = useState("");
  const [port, setPort] = useState("");
  const [pg, setPg] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  /** 第二个对外端口（部分服务发布两个，如 ARTEX）；空串 = 不显示/不提交 */
  const [port2, setPort2] = useState("");
  const [hasPort2, setHasPort2] = useState(false);

  useEffect(() => {
    void invoke<DockerServiceConfig>("docker_service_config", { projectId: project.id })
      .then((c) => {
        setDir(c.dir);
        setPort(String(c.port));
        // 服务不发布第二个端口时（c.port2 = 0）就别显示那个输入框，
        // 否则会给用户一个改了却不起作用的字段
        setHasPort2((c.port2 ?? 0) > 0);
        setPort2(c.port2 > 0 ? String(c.port2) : "");
        setPg(c.pgPassword);
      })
      .catch((e) => setMsg({ ok: false, text: String(e) }));
  }, [project.id]);

  const save = async () => {
    const p = Math.round(Number(port));
    if (!Number.isFinite(p) || p <= 0) {
      setMsg({ ok: false, text: t("nodeproj.configPortInvalid") });
      return;
    }
    const p2 = Math.round(Number(port2 || 0));
    if (port2.trim() && (!Number.isFinite(p2) || p2 <= 0)) {
      setMsg({ ok: false, text: t("nodeproj.configPortInvalid") });
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      const c = await invoke<DockerServiceConfig>("docker_save_service_config", {
        projectId: project.id,
        port: p,
        port2: p2 || null,
        pgPassword: pg,
      });
      setPort(String(c.port));
      setHasPort2((c.port2 ?? 0) > 0);
      setPort2(c.port2 > 0 ? String(c.port2) : "");
      setPg(c.pgPassword);
      setMsg({ ok: true, text: t("nodeproj.configSaved") });
      onSaved();
    } catch (e) {
      setMsg({ ok: false, text: typeof e === "string" ? e : String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <SharedModal
      open
      onClose={onClose}
      title={t("nodeproj.configTitle", { name: project.displayName })}
      width={520}
      footer={
        <>
          <SharedButton onClick={onClose} variant="secondary">
            {t("common.cancel")}
          </SharedButton>
          <SharedButton onClick={() => void save()} variant="primary" disabled={busy}>
            {t("common.save")}
          </SharedButton>
        </>
      }
    >
      <div className="space-y-3 text-body">
        <label className="block space-y-1">
          <span className="text-slate-400">{t("nodeproj.configPort")}</span>
          <input
            value={port}
            onChange={(e) => setPort(e.target.value)}
            inputMode="numeric"
            className="w-full rounded-ctl bg-white/5 border border-white/10 px-2.5 py-1.5 text-slate-100 outline-none focus:border-[var(--module-accent-ring)]"
          />
        </label>
        {hasPort2 && (
          <label className="block space-y-1">
            <span className="text-slate-400">{t("nodeproj.configPort2")}</span>
            <input
              value={port2}
              onChange={(e) => setPort2(e.target.value)}
              inputMode="numeric"
              className="w-full rounded-ctl bg-white/5 border border-white/10 px-2.5 py-1.5 text-slate-100 outline-none focus:border-[var(--module-accent-ring)]"
            />
          </label>
        )}
        <label className="block space-y-1">
          <span className="text-slate-400">{t("nodeproj.configPgPassword")}</span>
          <input
            value={pg}
            onChange={(e) => setPg(e.target.value)}
            spellCheck={false}
            className="w-full rounded-ctl bg-white/5 border border-white/10 px-2.5 py-1.5 font-mono text-slate-100 outline-none focus:border-[var(--module-accent-ring)]"
          />
        </label>
        {dir && <div className="break-all font-mono text-tiny text-slate-500">{dir}</div>}
        <Note tone="warn" title={t("nodeproj.configPwWarnTitle")}>
          <p>{t("nodeproj.configPwWarn")}</p>
        </Note>
        <p className="text-tiny text-slate-500">{t("nodeproj.configApplyHint")}</p>
        {msg && (
          <div className={`text-tiny ${msg.ok ? "text-emerald-400" : "text-red-400"}`}>{msg.text}</div>
        )}
      </div>
    </SharedModal>
  );
}
