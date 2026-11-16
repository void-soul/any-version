import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Activity, Download, Eye, Files, FolderOpen, GitBranch, Monitor, Network, Pencil, Play, Plus, RefreshCw, Settings2, ShieldCheck, TerminalSquare, Trash2, Upload, X } from "lucide-react";
import { TrustedHostsDialog } from "../ssh/TrustedHostsDialog";
import { TerminalSettingsDialog } from "../ssh/TerminalSettingsDialog";
import { GroupSidebar } from "./GroupSidebar";
import { SessionWorkspace } from "./SessionWorkspace";
import { ConnectionSwitcher } from "./ConnectionSwitcher";
import { buildGroups, filterConnections, groupOptions } from "./connectionGroups";
import { ignoreInvokeError } from "../../utils/invokeOrReport";
import { activateTab, closeTab, openTab, toggleSplit, type SessionKind, type TabsState } from "./sessionTabs";

interface RdpConnection {
  id: string; name: string; host: string; port: number; kind: string; username: string;
  password: string | null; finalshellCipher: string | null; finalshellRaw: string | null;
  authType: string; privateKeyPath: string; keyPassphrase: string;
  domain: string; note: string; group: string; source: string; createdAt: number; updatedAt: number;
}
interface ImportReport { imported: number; skipped: number; failed: number; messages: string[] }

const emptyConn = (): RdpConnection => ({
  id: "", name: "", host: "", port: 3389, kind: "rdp", username: "", password: null,
  finalshellCipher: null, finalshellRaw: null,
  authType: "password", privateKeyPath: "", keyPassphrase: "",
  domain: "", note: "", group: "", source: "local", createdAt: 0, updatedAt: 0,
});

/** 会话类型 → 标题后缀的 i18n key（终端不加后缀）。 */
function kindLabelKey(kind: SessionKind): string {
  switch (kind) {
    case "sftp": return "sftp.open";
    case "forward": return "forward.open";
    case "process": return "ssh.process";
    case "net": return "ssh.netlinks";
    default: return "ssh.openTerminal";
  }
}

const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-tiny border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";
const field = "w-full rounded border border-white/10 bg-black/30 px-2 py-1.5 text-tiny text-slate-100 outline-none focus:border-sky-500/60";

export default function RdpPanel() {
  const { t } = useTranslation();
  const [list, setList] = useState<RdpConnection[]>([]);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState("");
  const [editing, setEditing] = useState<RdpConnection | null>(null);
  const [passwordDraft, setPasswordDraft] = useState("");
  const [passphraseDraft, setPassphraseDraft] = useState("");
  const [tabs, setTabs] = useState<TabsState>({ tabs: [], activeId: null, splitId: null });
  /** 连接管理页：会话与列表不再互斥切换，管理页只是覆盖层 */
  const [manageVisible, setManageVisible] = useState(false);
  /** 同一主机多开终端时给 tab id 加序号，避免与已有会话冲突 */
  const sessionSeq = useRef(0);
  const [groupFilter, setGroupFilter] = useState<string | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({});
  const [showTrusted, setShowTrusted] = useState(false);
  const [showTerminalSettings, setShowTerminalSettings] = useState(false);
  const [keyList, setKeyList] = useState<string[]>([]);

  const load = useCallback(async () => {
    try { setList(await invoke<RdpConnection[]>("rdp_list")); }
    catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  }, [t]);

  useEffect(() => { void load(); }, [load]);

  const report = (r: ImportReport) => {
    const tail = r.messages.length ? ` · ${r.messages.join(" / ")}` : "";
    setToast(t("rdp.importOk", { imported: r.imported, skipped: r.skipped, failed: r.failed }) + tail);
    void load();
  };

  const importFs = async (directory: boolean) => {
    try {
      const picked = await openDialog(directory
        ? { directory: true, multiple: false }
        : { multiple: true, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (!picked) return;
      setBusy(true);
      const paths = Array.isArray(picked) ? picked : [picked];
      report(await invoke<ImportReport>("rdp_import_finalshell", { paths }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
    finally { setBusy(false); }
  };

  const exportFs = async () => {
    try {
      const dir = await openDialog({ directory: true, multiple: false });
      if (!dir || typeof dir !== "string") return;
      setBusy(true);
      const n = await invoke<number>("rdp_export_finalshell", { dir, ids: [] });
      setToast(t("rdp.exportOk", { count: n }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
    finally { setBusy(false); }
  };

  const exportBackup = async () => {
    try {
      const path = await saveDialog({
        title: t("rdp.exportBackup"),
        defaultPath: `rdp-connections-${new Date().toISOString().slice(0, 10)}.json`,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path || typeof path !== "string") return;
      setBusy(true);
      const n = await invoke<number>("rdp_export_backup", { path });
      setToast(t("rdp.exportOk", { count: n }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
    finally { setBusy(false); }
  };

  const importBackup = async () => {
    try {
      const picked = await openDialog({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (!picked || typeof picked !== "string") return;
      setBusy(true);
      report(await invoke<ImportReport>("rdp_import_backup", { path: picked }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
    finally { setBusy(false); }
  };

  const save = async () => {
    if (!editing) return;
    try {
      setBusy(true);
      const isFs = editing.source === "finalshell";
      // FinalShell 连接的密码走它自己的算法，不能塞进自有的 ENC_V2: 字段。
      const conn = {
        ...editing,
        password: isFs ? editing.password : (passwordDraft.trim() ? passwordDraft : editing.password),
        keyPassphrase: passphraseDraft || editing.keyPassphrase,
      };
      let next = await invoke<RdpConnection[]>("rdp_save", { conn });
      if (isFs && passwordDraft.trim()) {
        next = await invoke<RdpConnection[]>("rdp_set_finalshell_password", { id: editing.id, plain: passwordDraft });
      }
      setList(next);
      setEditing(null);
      setPasswordDraft("");
      setPassphraseDraft("");
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
    finally { setBusy(false); }
  };

  const pickKeyPath = async () => {
    const picked = await openDialog({ multiple: false });
    if (picked && typeof picked === "string" && editing) {
      patch("privateKeyPath", picked);
    }
  };

  /** 导入私钥到应用密钥库（复制一份，原文件不动）。 */
  const importKey = async () => {
    const picked = await openDialog({ multiple: false });
    if (!picked || typeof picked !== "string") return;
    try {
      const stored = await invoke<string>("ssh_import_key", { src: picked });
      setKeyList(await invoke<string[]>("ssh_list_keys"));
      patch("privateKeyPath", stored);
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  const moveToGroup = async (connId: string, group: string) => {
    const target = list.find((c) => c.id === connId);
    if (!target || target.group === group) return;
    try {
      setList(await invoke<RdpConnection[]>("rdp_save", { conn: { ...target, group } }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  const revealPassword = async () => {
    if (!editing?.id || !editing.finalshellCipher) return;
    try {
      setBusy(true);
      setPasswordDraft(await invoke<string>("rdp_reveal_password", { id: editing.id }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
    finally { setBusy(false); }
  };

  const remove = async (id: string) => {
    if (!window.confirm(t("rdp.confirmDelete"))) return;
    try { setList(await invoke<RdpConnection[]>("rdp_delete", { id })); }
    catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  const launch = async (id: string) => {
    try {
      // 后端在「无法自动填密码」时返回原因，需如实告知，不能静默降级
      const warning = await invoke<string | null>("rdp_launch", { id });
      if (warning) setToast(warning);
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  const openEditor = (conn?: RdpConnection) => {
    setEditing(conn ? { ...conn } : emptyConn());
    setPasswordDraft("");
    setPassphraseDraft("");
    // 打开表单时刷新密钥库列表，供私钥下拉选择
    void invoke<string[]>("ssh_list_keys")
      .then(setKeyList)
      .catch((e) => ignoreInvokeError(e, "密钥库列表加载失败不影响手动填写路径"));
  };
  const patch = (key: keyof RdpConnection, value: string | number) =>
    setEditing((prev) => prev ? { ...prev, [key]: value } : prev);

  const changeKind = (kind: string) =>
    setEditing((prev) => prev ? {
      ...prev,
      kind,
      // 只在端口还是另一类型的默认值时联动，避免覆盖用户自定义端口
      port: prev.port === (kind === "ssh" ? 3389 : 22) ? (kind === "ssh" ? 22 : 3389) : prev.port,
    } : prev);

  /** 打开会话：终端允许同一主机多开（id 带序号），文件/转发复用已有标签。 */
  const openSession = (conn: RdpConnection, kind: SessionKind) => {
    const title = `${conn.name} · ${conn.username}@${conn.host}`;
    const forceNew = kind === "terminal";
    const id = forceNew
      ? `${kind}_${conn.id}_${++sessionSeq.current}`
      : `${kind}_${conn.id}`;
    setTabs((prev) => openTab(prev, {
      id,
      connectionId: conn.id,
      title: kind === "terminal" ? title : `${title} · ${t(kindLabelKey(kind))}`,
      kind,
    }, forceNew));
    setManageVisible(false);
  };

  const hasTabs = tabs.tabs.length > 0;
  const activeTab = tabs.tabs.find((tab) => tab.id === tabs.activeId) ?? null;
  const activeConnection = activeTab
    ? list.find((conn) => conn.id === activeTab.connectionId) ?? null
    : null;

  const listView = (
    <div className="flex h-full flex-col gap-3 p-4 text-slate-200">
      <div className="flex flex-wrap items-center gap-2">
        <Monitor className="h-5 w-5 text-sky-400" />
        <h2 className="text-title font-semibold">{t("rdp.title")}</h2>
        <span className="text-tiny text-slate-500">{t("rdp.encryptedNote")}</span>
        <div className="ml-auto flex flex-wrap gap-1.5">
          {hasTabs && (
            <button className={btn} onClick={() => setManageVisible(false)}>
              ← {t("ssh.backToSession", { count: tabs.tabs.length })}
            </button>
          )}
          <button className={btn} disabled={busy} onClick={() => openEditor()}><Plus className="h-3.5 w-3.5" />{t("rdp.add")}</button>
          <button className={btn} disabled={busy} onClick={() => void importFs(false)}><Upload className="h-3.5 w-3.5" />{t("rdp.importFiles")}</button>
          <button className={btn} disabled={busy} onClick={() => void importFs(true)}><FolderOpen className="h-3.5 w-3.5" />{t("rdp.importDir")}</button>
          <button className={btn} disabled={busy} onClick={() => void exportFs()}><Download className="h-3.5 w-3.5" />{t("rdp.exportFs")}</button>
          <button className={btn} disabled={busy} onClick={() => void exportBackup()}>{t("rdp.exportBackup")}</button>
          <button className={btn} disabled={busy} onClick={() => void importBackup()}>{t("rdp.importBackup")}</button>
          <button className={btn} disabled={busy} onClick={() => void load()}><RefreshCw className="h-3.5 w-3.5" />{t("rdp.refresh")}</button>
          <button className={btn} title={t("trust.title")} onClick={() => setShowTrusted(true)}><ShieldCheck className="h-3.5 w-3.5" /></button>
          <button className={btn} title={t("terminalSettings.title")} onClick={() => setShowTerminalSettings(true)}><Settings2 className="h-3.5 w-3.5" /></button>
        </div>
      </div>

      {showTrusted && <TrustedHostsDialog onClose={() => setShowTrusted(false)} />}
      {showTerminalSettings && <TerminalSettingsDialog onClose={() => setShowTerminalSettings(false)} />}

      {toast && <div className="rounded border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-tiny text-amber-200">{toast}</div>}

      <div className="flex min-h-0 flex-1 gap-2">
        <GroupSidebar
          groups={buildGroups(list)}
          total={list.length}
          selected={groupFilter}
          collapsed={collapsedGroups}
          onSelect={setGroupFilter}
          onToggle={(key) => setCollapsedGroups((prev) => ({ ...prev, [key]: !prev[key] }))}
          onMove={(id, group) => void moveToGroup(id, group)}
        />
        <div className="min-h-0 flex-1 overflow-auto rounded border border-white/10">
        <table className="w-full text-tiny">
          <thead className="sticky top-0 bg-slate-900/90 text-slate-400">
            <tr>
              <th className="px-3 py-2 text-left">{t("rdp.kind")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.name")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.host")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.port")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.username")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.source")}</th>
              <th className="px-3 py-2 text-left">{t("rdp.actions")}</th>
            </tr>
          </thead>
          <tbody>
            {list.length === 0 && <tr><td colSpan={7} className="px-3 py-8 text-center text-slate-500">{t("rdp.empty")}</td></tr>}
            {filterConnections(list, groupFilter).map((c) => (
              <tr key={c.id} className="border-t border-white/5 hover:bg-white/5">
                <td className="px-3 py-2 text-slate-400">{c.kind === "ssh" ? t("rdp.kindSsh") : t("rdp.kindRdp")}</td>
                <td className="px-3 py-2"><div className="font-medium">{c.name}</div>{c.note && <div className="text-slate-500">{c.note}</div>}</td>
                <td className="px-3 py-2">{c.host}</td>
                <td className="px-3 py-2">{c.port}</td>
                <td className="px-3 py-2">{c.username}</td>
                <td className="px-3 py-2 text-slate-400">{c.source === "finalshell" ? t("rdp.fromFs") : t("rdp.fromLocal")}</td>
                <td className="px-3 py-2">
                  <div className="flex gap-1">
                    {c.kind === "ssh" ? (
                      <>
                        <button className={btn} title={t("ssh.openTerminal")} onClick={() => openSession(c, "terminal")}><TerminalSquare className="h-3 w-3" /></button>
                        <button className={btn} title={t("sftp.open")} onClick={() => openSession(c, "sftp")}><Files className="h-3 w-3" /></button>
                        <button className={btn} title={t("forward.open")} onClick={() => openSession(c, "forward")}><GitBranch className="h-3 w-3" /></button>
                        <button className={btn} title={t("ssh.process")} onClick={() => openSession(c, "process")}><Activity className="h-3 w-3" /></button>
                        <button className={btn} title={t("ssh.netlinks")} onClick={() => openSession(c, "net")}><Network className="h-3 w-3" /></button>
                      </>
                    ) : (
                      <button className={btn} title={t("rdp.launch")} onClick={() => void launch(c.id)}><Play className="h-3 w-3" /></button>
                    )}
                    <button className={btn} title={t("rdp.edit")} onClick={() => openEditor(c)}><Pencil className="h-3 w-3" /></button>
                    <button className={btn} title={t("rdp.delete")} onClick={() => void remove(c.id)}><Trash2 className="h-3 w-3" /></button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        </div>
      </div>

      {editing && (
        <div className="fixed inset-0 z-[130] flex items-center justify-center bg-black/70 p-4">
          <div className="w-[440px] max-w-full rounded border border-white/10 bg-slate-900 p-4 shadow-2xl">
            <div className="mb-3 flex items-center justify-between">
              <h3 className="text-caption font-semibold">{editing.id ? t("rdp.edit") : t("rdp.add")}</h3>
              <button className={btn} onClick={() => setEditing(null)}><X className="h-3.5 w-3.5" /></button>
            </div>
            <div className="space-y-2">
              <label className="block text-tiny text-slate-400">{t("rdp.kind")}
                <select className={field} value={editing.kind} onChange={(e) => changeKind(e.target.value)}>
                  <option value="rdp">{t("rdp.kindRdp")}</option>
                  <option value="ssh">{t("rdp.kindSsh")}</option>
                </select>
              </label>
              <label className="block text-tiny text-slate-400">{t("rdp.name")}<input className={field} value={editing.name} onChange={(e) => patch("name", e.target.value)} /></label>
              <div className="grid grid-cols-[1fr_90px] gap-2">
                <label className="block text-tiny text-slate-400">{t("rdp.host")}<input className={field} value={editing.host} onChange={(e) => patch("host", e.target.value)} /></label>
                <label className="block text-tiny text-slate-400">{t("rdp.port")}<input className={field} type="number" value={editing.port} onChange={(e) => patch("port", Number(e.target.value))} /></label>
              </div>
              <label className="block text-tiny text-slate-400">{t("rdp.username")}<input className={field} value={editing.username} onChange={(e) => patch("username", e.target.value)} /></label>
              {editing.kind === "ssh" && (
                <>
                  <label className="block text-tiny text-slate-400">{t("rdp.authType")}
                    <select className={field} value={editing.authType} onChange={(e) => patch("authType", e.target.value)}>
                      <option value="password">{t("rdp.authPassword")}</option>
                      <option value="key">{t("rdp.authKey")}</option>
                    </select>
                  </label>
                  {editing.authType === "key" && (
                    <>
                      <label className="block text-tiny text-slate-400">{t("rdp.keyPath")}
                        <div className="flex gap-1">
                          <input className={field} value={editing.privateKeyPath} onChange={(e) => patch("privateKeyPath", e.target.value)} placeholder="id_ed25519 / id_rsa" />
                          <button className={btn} title={t("rdp.pickKeyFile")} onClick={() => void pickKeyPath()}><FolderOpen className="h-3.5 w-3.5" /></button>
                          <button className={btn} title={t("rdp.importKey")} onClick={() => void importKey()}>{t("rdp.importKey")}</button>
                        </div>
                        {keyList.length > 0 && (
                          <select className={`${field} mt-1`} value="" onChange={(e) => e.target.value && patch("privateKeyPath", e.target.value)}>
                            <option value="">{t("rdp.pickFromKeyStore")}</option>
                            {keyList.map((k) => <option key={k} value={k}>{k.split(/[\\/]/).pop()}</option>)}
                          </select>
                        )}
                      </label>
                      <label className="block text-tiny text-slate-400">{t("rdp.keyPassphrase")}
                        <input className={field} type="password" autoComplete="new-password" value={passphraseDraft} onChange={(e) => setPassphraseDraft(e.target.value)} />
                      </label>
                    </>
                  )}
                </>
              )}
              {editing.kind === "rdp" && (
                <label className="block text-tiny text-slate-400">{t("rdp.domain")}<input className={field} value={editing.domain} onChange={(e) => patch("domain", e.target.value)} /></label>
              )}
              <label className="block text-tiny text-slate-400">{t("rdp.password")}
                <div className="flex gap-1">
                  <input className={field} type="password" autoComplete="new-password" placeholder={t("rdp.passwordPlaceholder")} value={passwordDraft} onChange={(e) => setPasswordDraft(e.target.value)} />
                  {editing.finalshellCipher && (
                    <button className={btn} disabled={busy} title={t("rdp.revealPassword")} onClick={() => void revealPassword()}><Eye className="h-3.5 w-3.5" /></button>
                  )}
                </div>
              </label>
              {editing.finalshellCipher && <div className="text-micro text-amber-300">{t("rdp.fsCipherNote")}</div>}
              <label className="block text-tiny text-slate-400">{t("rdp.group")}
                <input className={field} list="rdp-group-options" value={editing.group} onChange={(e) => patch("group", e.target.value)} placeholder={t("rdp.groupPlaceholder")} />
                <datalist id="rdp-group-options">
                  {groupOptions(list).map((g) => <option key={g} value={g} />)}
                </datalist>
              </label>
              <label className="block text-tiny text-slate-400">{t("rdp.note")}<input className={field} value={editing.note} onChange={(e) => patch("note", e.target.value)} /></label>
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button className={btn} onClick={() => setEditing(null)}>{t("rdp.cancel")}</button>
              <button className={btn} disabled={busy} onClick={() => void save()}>{t("rdp.save")}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );

  if (manageVisible) return listView;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b border-white/10 px-3 py-2">
        <ConnectionSwitcher
          connections={list}
          active={activeConnection}
          onPick={(picked) => {
            // 切换器只依赖连接的最小字段；这里取回完整连接再开会话
            const conn = list.find((item) => item.id === picked.id);
            if (!conn) return;
            if (conn.kind === "ssh") openSession(conn, "terminal");
            else void launch(conn.id);
          }}
          onManage={() => setManageVisible(true)}
        />
        <span className="text-micro text-slate-500">{t("tabs.hint")}</span>
        <div className="ml-auto flex gap-1.5">
          <button className={btn} title={t("trust.title")} onClick={() => setShowTrusted(true)}><ShieldCheck className="h-3.5 w-3.5" /></button>
          <button className={btn} title={t("terminalSettings.title")} onClick={() => setShowTerminalSettings(true)}><Settings2 className="h-3.5 w-3.5" /></button>
          <button className={btn} onClick={() => void load()}><RefreshCw className="h-3.5 w-3.5" /></button>
        </div>
      </div>
      {hasTabs ? (
        <div className="min-h-0 flex-1">
          <SessionWorkspace
            state={tabs}
            titleFor={(tab) => tab.title}
            onActivate={(id) => setTabs((prev) => activateTab(prev, id))}
            onClose={(id) => setTabs((prev) => closeTab(prev, id))}
            onToggleSplit={() => setTabs((prev) => toggleSplit(prev))}
            onOpenMonitor={(connectionId, view) => {
              const conn = list.find((item) => item.id === connectionId);
              if (conn) openSession(conn, view);
            }}
          />
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center text-tiny text-slate-500">{t("rdp.noSessions")}</div>
      )}
    </div>
  );
}
