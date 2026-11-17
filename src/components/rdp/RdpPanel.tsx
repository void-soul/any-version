import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Activity, Download, Eye, Files, FolderOpen, GitBranch, Monitor, Network, Pencil, Play, Plus, RefreshCw, Settings2, ShieldCheck, TerminalSquare, Trash2, Upload, X } from "lucide-react";
import { TrustedHostsDialog } from "../ssh/TrustedHostsDialog";
import { TerminalSettingsDialog } from "../ssh/TerminalSettingsDialog";
import { ConnectionTree } from "./ConnectionTree";
import { SessionWorkspace } from "./SessionWorkspace";
import { ConnectionSwitcher } from "./ConnectionSwitcher";
import { normalizePath } from "./groupTree";
import { ignoreInvokeError } from "../../utils/invokeOrReport";
import { activateTab, closeTab, openTab, toggleSplit, type SessionKind, type TabsState } from "./sessionTabs";

interface RdpConnection {
  id: string; name: string; host: string; port: number; kind: string; username: string;
  password: string | null; finalshellCipher: string | null; finalshellRaw: string | null;
  authType: string; privateKeyPath: string; keyPassphrase: string;
  domain: string; mapDrives: boolean; clipboard: boolean; note: string; group: string; source: string; createdAt: number; updatedAt: number;
}
interface ImportReport { imported: number; skipped: number; failed: number; messages: string[] }

const emptyConn = (): RdpConnection => ({
  id: "", name: "", host: "", port: 3389, kind: "rdp", username: "", password: null,
  finalshellCipher: null, finalshellRaw: null,
  authType: "password", privateKeyPath: "", keyPassphrase: "",
  // 驱动器映射与剪贴板默认开启（对齐 FinalShell）
  domain: "", mapDrives: true, clipboard: true, note: "", group: "", source: "local", createdAt: 0, updatedAt: 0,
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

// 字号按 App.css 的语义：按钮默认 caption(11px)，输入框 body(12px)，不再压到 tiny(10px)
const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-caption border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10 disabled:opacity-40";
const field = "w-full rounded border border-white/10 bg-black/30 px-2 py-1.5 text-body text-slate-100 outline-none focus:border-sky-500/60";

export default function RdpPanel() {
  const { t } = useTranslation();
  const [list, setList] = useState<RdpConnection[]>([]);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState("");

  // 提示需要有生命周期：否则「已刷新」这类提示会永久停在顶部
  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(""), 4000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  /** 刷新连接列表并给出可见反馈（原来的刷新成功时没有任何提示，看起来像没反应）。 */
  const refreshWithToast = useCallback(async () => {
    try {
      setList(await invoke<RdpConnection[]>("rdp_list"));
      setFolders(await invoke<string[]>("rdp_groups_list"));
      setToast(t("rdp.refreshed"));
    } catch (e) {
      setToast(t("rdp.fail", { err: String(e) }));
    }
  }, [t]);
  const [editing, setEditing] = useState<RdpConnection | null>(null);
  const [passwordDraft, setPasswordDraft] = useState("");
  const [passphraseDraft, setPassphraseDraft] = useState("");
  const [tabs, setTabs] = useState<TabsState>({ tabs: [], activeId: null, splitId: null });
  /** 连接管理页：会话与列表不再互斥切换，管理页只是覆盖层 */
  const [manageVisible, setManageVisible] = useState(false);
  /** 同一主机多开终端时给 tab id 加序号，避免与已有会话冲突 */
  const sessionSeq = useRef(0);
  /** 空文件夹清单（连接为空的文件夹只能靠它存在）。 */
  const [folders, setFolders] = useState<string[]>([]);
  const [showTrusted, setShowTrusted] = useState(false);
  const [showTerminalSettings, setShowTerminalSettings] = useState(false);
  const [keyList, setKeyList] = useState<string[]>([]);

  const load = useCallback(async () => {
    try { setList(await invoke<RdpConnection[]>("rdp_list")); }
    catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  }, [t]);

  /** 空文件夹清单与连接一起加载：只靠连接推导会丢掉没有内容的文件夹。 */
  const loadFolders = useCallback(async () => {
    try { setFolders(await invoke<string[]>("rdp_groups_list")); }
    catch (e) { ignoreInvokeError(e, "文件夹清单加载失败不影响连接列表"); }
  }, []);

  useEffect(() => { void load(); void loadFolders(); }, [load, loadFolders]);

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

  /** 拖拽连接 → 文件夹：只改这条连接的 `group` 路径。 */
  const moveConnection = async (connId: string, folder: string) => {
    const target = list.find((c) => c.id === connId);
    const next = normalizePath(folder);
    if (!target || normalizePath(target.group) === next) return;
    try {
      setList(await invoke<RdpConnection[]>("rdp_save", { conn: { ...target, group: next } }));
    } catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  type FolderState = { connections: RdpConnection[]; folders: string[] };
  const applyFolderState = (state: FolderState) => {
    setList(state.connections);
    setFolders(state.folders);
  };

  const createFolder = async (path: string) => {
    try { applyFolderState(await invoke<FolderState>("rdp_folder_create", { path })); }
    catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  const renameFolder = async (from: string, to: string) => {
    try { applyFolderState(await invoke<FolderState>("rdp_folder_rename", { from, to })); }
    catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
  };

  const deleteFolder = async (path: string) => {
    if (!window.confirm(t("folders.removeConfirm", { name: path }))) return;
    try { applyFolderState(await invoke<FolderState>("rdp_folder_delete", { path })); }
    catch (e) { setToast(t("rdp.fail", { err: String(e) })); }
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
  const patch = (key: keyof RdpConnection, value: string | number | boolean) =>
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
          <button className={btn} disabled={busy} onClick={() => void refreshWithToast()}><RefreshCw className="h-3.5 w-3.5" />{t("rdp.refresh")}</button>
          <button className={btn} title={t("trust.title")} onClick={() => setShowTrusted(true)}><ShieldCheck className="h-3.5 w-3.5" /></button>
          <button className={btn} title={t("terminalSettings.title")} onClick={() => setShowTerminalSettings(true)}><Settings2 className="h-3.5 w-3.5" /></button>
        </div>
      </div>

      {showTrusted && <TrustedHostsDialog onClose={() => setShowTrusted(false)} />}
      {showTerminalSettings && <TerminalSettingsDialog onClose={() => setShowTerminalSettings(false)} />}

      {toast && <div className="rounded border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-caption text-amber-200">{toast}</div>}

      <ConnectionTree
        connections={list}
        folders={folders}
        emptyText={t("rdp.empty")}
        onMoveConnection={(id, folder) => void moveConnection(id, folder)}
        onMoveFolder={(from, to) => void renameFolder(from, to)}
        onRenameFolder={(from, to) => void renameFolder(from, to)}
        onDeleteFolder={(path) => void deleteFolder(path)}
        onCreateFolder={(path) => void createFolder(path)}
        renderActions={(c) => {
          const conn = list.find((item) => item.id === c.id);
          if (!conn) return null;
          return (
            <div className="flex gap-1">
              {conn.kind === "ssh" ? (
                <>
                  <button className={btn} title={t("ssh.openTerminal")} onClick={() => openSession(conn, "terminal")}><TerminalSquare className="h-3 w-3" /></button>
                  <button className={btn} title={t("sftp.open")} onClick={() => openSession(conn, "sftp")}><Files className="h-3 w-3" /></button>
                  <button className={btn} title={t("forward.open")} onClick={() => openSession(conn, "forward")}><GitBranch className="h-3 w-3" /></button>
                  <button className={btn} title={t("ssh.process")} onClick={() => openSession(conn, "process")}><Activity className="h-3 w-3" /></button>
                  <button className={btn} title={t("ssh.netlinks")} onClick={() => openSession(conn, "net")}><Network className="h-3 w-3" /></button>
                </>
              ) : (
                <button className={btn} title={t("rdp.launch")} onClick={() => void launch(conn.id)}><Play className="h-3 w-3" /></button>
              )}
              <button className={btn} title={t("rdp.edit")} onClick={() => openEditor(conn)}><Pencil className="h-3 w-3" /></button>
              <button className={btn} title={t("rdp.delete")} onClick={() => void remove(conn.id)}><Trash2 className="h-3 w-3" /></button>
            </div>
          );
        }}
      />

      {editing && (
        <div className="fixed inset-0 z-[130] flex items-center justify-center bg-black/70 p-4">
          <div className="w-[440px] max-w-full rounded border border-white/10 bg-slate-900 p-4 shadow-2xl">
            <div className="mb-3 flex items-center justify-between">
              <h3 className="text-title font-semibold">{editing.id ? t("rdp.edit") : t("rdp.add")}</h3>
              <button className={btn} onClick={() => setEditing(null)}><X className="h-3.5 w-3.5" /></button>
            </div>
            <div className="space-y-2">
              <label className="block text-caption text-slate-400">{t("rdp.kind")}
                <select className={field} value={editing.kind} onChange={(e) => changeKind(e.target.value)}>
                  <option value="rdp">{t("rdp.kindRdp")}</option>
                  <option value="ssh">{t("rdp.kindSsh")}</option>
                </select>
              </label>
              <label className="block text-caption text-slate-400">{t("rdp.name")}<input className={field} value={editing.name} onChange={(e) => patch("name", e.target.value)} /></label>
              <div className="grid grid-cols-[1fr_90px] gap-2">
                <label className="block text-caption text-slate-400">{t("rdp.host")}<input className={field} value={editing.host} onChange={(e) => patch("host", e.target.value)} /></label>
                <label className="block text-caption text-slate-400">{t("rdp.port")}<input className={field} type="number" value={editing.port} onChange={(e) => patch("port", Number(e.target.value))} /></label>
              </div>
              <label className="block text-caption text-slate-400">{t("rdp.username")}<input className={field} value={editing.username} onChange={(e) => patch("username", e.target.value)} /></label>
              {editing.kind === "ssh" && (
                <>
                  <label className="block text-caption text-slate-400">{t("rdp.authType")}
                    <select className={field} value={editing.authType} onChange={(e) => patch("authType", e.target.value)}>
                      <option value="password">{t("rdp.authPassword")}</option>
                      <option value="key">{t("rdp.authKey")}</option>
                    </select>
                  </label>
                  {editing.authType === "key" && (
                    <>
                      <label className="block text-caption text-slate-400">{t("rdp.keyPath")}
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
                      <label className="block text-caption text-slate-400">{t("rdp.keyPassphrase")}
                        <input className={field} type="password" autoComplete="new-password" value={passphraseDraft} onChange={(e) => setPassphraseDraft(e.target.value)} />
                      </label>
                    </>
                  )}
                </>
              )}
              {editing.kind === "rdp" && (
                <>
                  <label className="block text-caption text-slate-400">{t("rdp.domain")}<input className={field} value={editing.domain} onChange={(e) => patch("domain", e.target.value)} /></label>
                  <label className="flex items-center gap-2 text-caption text-slate-300">
                    <input
                      type="checkbox"
                      checked={editing.mapDrives}
                      onChange={(e) => patch("mapDrives", e.target.checked)}
                    />
                    {t("rdp.mapDrives")}
                  </label>
                  <label className="flex items-center gap-2 text-caption text-slate-300">
                    <input
                      type="checkbox"
                      checked={editing.clipboard}
                      onChange={(e) => patch("clipboard", e.target.checked)}
                    />
                    {t("rdp.clipboard")}
                  </label>
                  <div className="text-tiny text-slate-500">{t("rdp.mapDrivesHint")}</div>
                </>
              )}
              <label className="block text-caption text-slate-400">{t("rdp.password")}
                <div className="flex gap-1">
                  <input className={field} type="password" autoComplete="new-password" placeholder={t("rdp.passwordPlaceholder")} value={passwordDraft} onChange={(e) => setPasswordDraft(e.target.value)} />
                  {editing.finalshellCipher && (
                    <button className={btn} disabled={busy} title={t("rdp.revealPassword")} onClick={() => void revealPassword()}><Eye className="h-3.5 w-3.5" /></button>
                  )}
                </div>
              </label>
              {editing.finalshellCipher && <div className="text-tiny text-amber-300">{t("rdp.fsCipherNote")}</div>}
              <label className="block text-caption text-slate-400">{t("rdp.note")}<input className={field} value={editing.note} onChange={(e) => patch("note", e.target.value)} /></label>
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
    <div className="flex h-full min-h-0 flex-col text-slate-200">
      {/* 这两个对话框必须在每个分支都渲染，否则在会话视图里点按钮只会改 state 而不出界面 */}
      {showTrusted && <TrustedHostsDialog onClose={() => setShowTrusted(false)} />}
      {showTerminalSettings && <TerminalSettingsDialog onClose={() => setShowTerminalSettings(false)} />}
      {toast && <div className="mx-3 mt-2 rounded border border-amber-400/30 bg-amber-400/10 px-3 py-2 text-caption text-amber-200">{toast}</div>}
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
        <span className="text-tiny text-slate-500">{t("tabs.hint")}</span>
        <div className="ml-auto flex gap-1.5">
          <button className={btn} title={t("trust.title")} onClick={() => setShowTrusted(true)}><ShieldCheck className="h-3.5 w-3.5" /></button>
          <button className={btn} title={t("terminalSettings.title")} onClick={() => setShowTerminalSettings(true)}><Settings2 className="h-3.5 w-3.5" /></button>
          <button className={btn} title={t("rdp.refresh")} onClick={() => void refreshWithToast()}><RefreshCw className="h-3.5 w-3.5" /></button>
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
            onSplitBlocked={() => setToast(t("tabs.splitNeedsTwo"))}
            onOpenMonitor={(connectionId, view) => {
              const conn = list.find((item) => item.id === connectionId);
              if (conn) openSession(conn, view);
            }}
          />
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center text-caption text-slate-500">{t("rdp.noSessions")}</div>
      )}
    </div>
  );
}
