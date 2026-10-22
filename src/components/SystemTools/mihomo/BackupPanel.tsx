// 备份与恢复页 —— 本地 zip 导出/导入 + WebDAV 备份管理
// 后端命令（mihomo_export_local_backup / mihomo_import_local_backup /
// mihomo_webdav_backup|list|restore|delete）此前只封装在 mihomoApi，界面无任何入口，
// 换机时分类、订阅、覆写都带不走。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Archive, CloudUpload, Download, FolderOpen, RefreshCw, Save, Trash2, Upload } from "lucide-react";
import { mihomoApi } from "../mihomoApi";
import {
  Modal, SettingItem, btnDanger, btnPrimary, btnSec, calcTraffic, cardCls, inputCls, labelCls, tagCls,
} from "./ui";

type DavItem = { name: string; size?: number | null; lastModified?: string | null };

function pickName(v: any): string {
  return typeof v === "string" ? v : "";
}

export default function BackupPanel({ running }: { running: boolean }) {
  const { t } = useTranslation();
  const [url, setUrl] = useState("");
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [dir, setDir] = useState("");
  const [busy, setBusy] = useState("");
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  const [items, setItems] = useState<DavItem[]>([]);
  const [confirm, setConfirm] = useState<{ title: string; msg: string; onOk: () => void } | null>(null);

  const flash = (s: string) => { setMsg(s); setErr(""); };
  const fail = (s: string) => { setErr(s); setMsg(""); };

  const loadCfg = async () => {
    try {
      const a: any = await mihomoApi.getAppConfig();
      setUrl(a?.webdav_url || "");
      setUser(a?.webdav_user || "");
      setPass(a?.webdav_pass || "");
      setDir(a?.webdavDir || "");
    } catch (e: any) {
      fail(String(e));
    }
  };
  useEffect(() => { loadCfg(); }, []);

  const saveCfg = async () => {
    setBusy("save");
    try {
      // webdav_url/user/pass 是 AppConfig 的 snake_case 字段；目录名走 extra（webdavDir）
      await mihomoApi.patchAppConfig({ webdav_url: url.trim(), webdav_user: user.trim(), webdav_pass: pass, webdavDir: dir.trim() });
      flash(t("mihomobackup.saved"));
    } catch (e: any) {
      fail(t("mihomobackup.saveFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doExport = async () => {
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const p = await saveDialog({
      title: t("mihomobackup.exportDialog"),
      defaultPath: `any-version-mihomo-${stamp}.zip`,
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });
    if (!p) return;
    setBusy("export");
    try {
      const saved = await mihomoApi.exportLocalBackup(p);
      flash(t("mihomobackup.exportOk", { path: saved }));
    } catch (e: any) {
      fail(t("mihomobackup.exportFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doImport = async () => {
    const p = await openDialog({
      title: t("mihomobackup.importDialog"),
      multiple: false,
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });
    if (typeof p !== "string" || !p) return;
    setConfirm({
      title: t("mihomobackup.importConfirmTitle"),
      msg: t("mihomobackup.importConfirm"),
      onOk: async () => {
        setBusy("import");
        try {
          await mihomoApi.importLocalBackup(p);
          if (running) await mihomoApi.restart();
          await loadCfg();
          flash(t("mihomobackup.importOk"));
        } catch (e: any) {
          fail(t("mihomobackup.importFailed", { err: String(e) }));
        } finally {
          setBusy("");
        }
      },
    });
  };

  const refreshList = async () => {
    setBusy("list");
    try {
      const r: any = await mihomoApi.webdavList();
      setItems(Array.isArray(r?.items) ? r.items : []);
      flash(t("mihomobackup.listOk", { count: Array.isArray(r?.items) ? r.items.length : 0 }));
    } catch (e: any) {
      setItems([]);
      fail(t("mihomobackup.listFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doBackup = async () => {
    setBusy("backup");
    try {
      const name = await mihomoApi.webdavBackup();
      flash(t("mihomobackup.backupOk", { name: pickName(name) }));
      await refreshList();
    } catch (e: any) {
      fail(t("mihomobackup.backupFailed", { err: String(e) }));
    } finally {
      setBusy("");
    }
  };

  const doRestore = (name: string) => {
    setConfirm({
      title: t("mihomobackup.restoreConfirmTitle"),
      msg: t("mihomobackup.restoreConfirm", { name }),
      onOk: async () => {
        setBusy(`restore:${name}`);
        try {
          await mihomoApi.webdavRestore(name);
          if (running) await mihomoApi.restart();
          await loadCfg();
          flash(t("mihomobackup.restored", { name }));
        } catch (e: any) {
          fail(t("mihomobackup.restoreFailed", { err: String(e) }));
        } finally {
          setBusy("");
        }
      },
    });
  };

  const doDelete = (name: string) => {
    setConfirm({
      title: t("mihomobackup.deleteConfirmTitle"),
      msg: t("mihomobackup.deleteConfirm", { name }),
      onOk: async () => {
        setBusy(`delete:${name}`);
        try {
          await mihomoApi.webdavDelete(name);
          await refreshList();
          flash(t("mihomobackup.deleted", { name }));
        } catch (e: any) {
          fail(t("mihomobackup.deleteFailed", { err: String(e) }));
        } finally {
          setBusy("");
        }
      },
    });
  };

  return (
    <div className="space-y-3">
      {msg && <div className="text-caption text-emerald-300 px-1 break-all">{msg}</div>}
      {err && <div className="text-caption text-rose-300 px-1 break-all">{err}</div>}

      {/* 本地备份 */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center gap-2 mb-1">
          <Archive className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-sm font-bold text-white">{t("mihomobackup.localTitle")}</h3>
        </div>
        <p className="text-caption text-slate-400 mb-2">{t("mihomobackup.localHint")}</p>
        <div className="flex flex-wrap items-center gap-2">
          <button className={btnPrimary} onClick={doExport} disabled={busy === "export"}>
            <span className="inline-flex items-center gap-1.5">
              <Download className={`w-3.5 h-3.5 ${busy === "export" ? "animate-bounce" : ""}`} />
              {busy === "export" ? t("mihomobackup.exporting") : t("mihomobackup.export")}
            </span>
          </button>
          <button className={btnSec} onClick={doImport} disabled={busy === "import"}>
            <span className="inline-flex items-center gap-1.5">
              <Upload className="w-3.5 h-3.5" />
              {busy === "import" ? t("mihomobackup.importing") : t("mihomobackup.import")}
            </span>
          </button>
          <button
            className={btnSec}
            onClick={() => mihomoApi.openPath("data").catch((e) => fail(String(e)))}
            title={t("mihomobackup.openDataDir")}
          >
            <span className="inline-flex items-center gap-1.5">
              <FolderOpen className="w-3.5 h-3.5" />{t("mihomobackup.openDataDir")}
            </span>
          </button>
        </div>
      </div>

      {/* WebDAV */}
      <div className={`${cardCls} p-4`}>
        <div className="flex items-center gap-2 mb-1">
          <CloudUpload className="w-4 h-4 text-sky-400" />
          <h3 className="text-sm font-bold text-white">{t("mihomobackup.webdavTitle")}</h3>
        </div>
        <p className="text-caption text-slate-400 mb-1">{t("mihomobackup.webdavHint")}</p>

        <div className="space-y-2 mt-2">
          <div>
            <label className={labelCls}>{t("mihomobackup.url")}</label>
            <input className={inputCls} value={url} placeholder="https://dav.example.com/dav"
              onChange={(e) => setUrl(e.target.value)} />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className={labelCls}>{t("mihomobackup.username")}</label>
              <input className={inputCls} value={user} onChange={(e) => setUser(e.target.value)} />
            </div>
            <div>
              <label className={labelCls}>{t("mihomobackup.password")}</label>
              <input className={inputCls} type="password" value={pass} onChange={(e) => setPass(e.target.value)} />
            </div>
          </div>
          <div>
            <label className={labelCls}>{t("mihomobackup.dir")}</label>
            <input className={inputCls} value={dir} placeholder="any-version" onChange={(e) => setDir(e.target.value)} />
            <div className="mt-1 text-tiny text-slate-500">{t("mihomobackup.dirHint")}</div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2 mt-3">
          <button className={btnSec} onClick={saveCfg} disabled={busy === "save"}>
            <span className="inline-flex items-center gap-1.5">
              <Save className="w-3.5 h-3.5" />{t("mihomobackup.save")}
            </span>
          </button>
          <button className={btnSec} onClick={refreshList} disabled={busy === "list"}>
            <span className="inline-flex items-center gap-1.5">
              <RefreshCw className={`w-3.5 h-3.5 ${busy === "list" ? "animate-spin" : ""}`} />
              {t("mihomobackup.refresh")}
            </span>
          </button>
          <button className={btnPrimary} onClick={doBackup} disabled={busy === "backup"}>
            <span className="inline-flex items-center gap-1.5">
              <CloudUpload className={`w-3.5 h-3.5 ${busy === "backup" ? "animate-bounce" : ""}`} />
              {busy === "backup" ? t("mihomobackup.backingUp") : t("mihomobackup.backupNow")}
            </span>
          </button>
          {!url.trim() && <span className="text-tiny text-amber-300">{t("mihomobackup.noConfigHint")}</span>}
        </div>

        {/* 备份列表 */}
        <div className="mt-3">
          <SettingItem title={t("mihomobackup.listTitle")} divider={false}>
            <span className={tagCls}>{items.length}</span>
          </SettingItem>
          <div className="rounded-card border border-white/10 divide-y divide-white/5 max-h-64 overflow-y-auto">
            {items.length === 0 && (
              <div className="px-3 py-3 text-caption text-slate-500">{t("mihomobackup.listEmpty")}</div>
            )}
            {items.map((it) => (
              <div key={it.name} className="px-3 py-2 flex items-center gap-2">
                <Archive className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="text-caption text-slate-200 truncate" title={it.name}>{it.name}</div>
                  <div className="text-tiny text-slate-500">
                    {typeof it.size === "number" ? calcTraffic(it.size) : "-"}
                    {it.lastModified ? ` · ${it.lastModified}` : ""}
                  </div>
                </div>
                <button
                  className={btnSec}
                  title={t("mihomobackup.restore")}
                  disabled={busy === `restore:${it.name}`}
                  onClick={() => doRestore(it.name)}
                >
                  <RefreshCw className={`w-3 h-3 ${busy === `restore:${it.name}` ? "animate-spin" : ""}`} />
                </button>
                <button
                  className={btnDanger}
                  title={t("mihomobackup.delete")}
                  disabled={busy === `delete:${it.name}`}
                  onClick={() => doDelete(it.name)}
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            ))}
          </div>
        </div>
      </div>

      {confirm && (
        <Modal
          title={confirm.title}
          onClose={() => setConfirm(null)}
          footer={
            <>
              <button className={btnSec} onClick={() => setConfirm(null)}>{t("mihomobackup.cancel")}</button>
              <button className={btnPrimary} onClick={() => { const c = confirm; setConfirm(null); c.onOk(); }}>
                {t("mihomobackup.confirm")}
              </button>
            </>
          }
        >
          <div className="text-sm text-slate-300 whitespace-pre-wrap leading-relaxed">{confirm.msg}</div>
        </Modal>
      )}
    </div>
  );
}
