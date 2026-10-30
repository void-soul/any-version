import { useState, useEffect, useCallback } from "react";
import { theamedAlert, alertError } from "./shared/ThemedAlert";
import { Plus, Search, Trash2, Pencil, KeyRound, Import, X, Loader2, ScanLine, Tag, Folder, FolderPlus } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { generateCode, type OtpCategory, type OtpToken } from "./OtpPanel.model";
import { TokenCard } from "./OtpPanel.TokenCard";
import { TokenForm } from "./OtpPanel.TokenForm";
import { CategoryModal } from "./OtpPanel.CategoryModal";
import { ConfirmModal } from "./OtpPanel.ConfirmModal";
export default function OtpPanel() {
  const { t } = useTranslation();
  const [tokens, setTokens] = useState<OtpToken[]>([]);
  const [search, setSearch] = useState("");
  const [now, setNow] = useState(Date.now());
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [editing, setEditing] = useState<OtpToken | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [importText, setImportText] = useState("");
  const [showImport, setShowImport] = useState(false);
  const [busy, setBusy] = useState(false);
  const [categories, setCategories] = useState<OtpCategory[]>([]);
  const [activeCategory, setActiveCategory] = useState<number | null>(null); // null = 全部
  // 分类新建/重命名弹窗：null=关闭；{mode:"add"} 新建；{mode:"rename",cat} 重命名
  const [catModal, setCatModal] = useState<{ mode: "add" } | { mode: "rename"; cat: OtpCategory } | null>(null);
  // 通用删除确认弹窗
  const [confirmModal, setConfirmModal] = useState<{
    title: string;
    message: string;
    danger?: boolean;
    onConfirm: () => void;
  } | null>(null);

  const load = useCallback(async () => {
    try {
      const list = await invoke<OtpToken[]>("otp_list");
      setTokens(list);
      const cats = await invoke<OtpCategory[]>("otp_list_categories");
      setCategories(cats);
    } catch (e) {
      console.error("加载 OTP 失败", e);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // 每秒刷新验证码与倒计时
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  const copyCode = async (token: OtpToken) => {
    const code = await generateCode(token, Date.now());
    try {
      // 复用剪贴板模块的「写入 → 隐藏 → 粘贴」通道：
      // 目标窗口取自唤起 Kira 前的活动窗口，避免把验证码粘回 OTP 面板。
      await invoke("clipboard_paste_text", { text: code });
      setCopiedId(token.id);
      setTimeout(() => setCopiedId(null), 1500);
      // 更新复制统计
      await invoke("otp_mark_copied", { id: token.id }).catch(() => {});
      load();
    } catch (e) {
      console.error("复制失败", e);
    }
  };

  // 扫码添加：选择图片 -> 后端识别二维码 -> 解析 otpauth URI -> 导入
  const scanQr = async () => {
    try {
      const selected = await openDialog({
        title: t("otp.scanPickTitle"),
        filters: [{ name: t("otp.pickImage"), extensions: ["png", "jpg", "jpeg", "bmp", "gif", "webp"] }],
      });
      if (!selected) return;
      setBusy(true);
      const text = await invoke<string>("otp_scan_qr", { path: selected });
      if (
        text.startsWith("otpauth://") ||
        text.startsWith("motp://") ||
        text.startsWith("otpauth-migration://")
      ) {
        const added = await invoke<OtpToken[]>("otp_import_uri", { text });
        load();
        if (added.length === 0) {
          theamedAlert(t("otp.scanNotOtpauth"));
        }
      } else {
        theamedAlert(t("otp.scanIgnored", { text }));
      }
    } catch (e) {
      alertError(t("otp.scanFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  // 分类操作
  const addCategory = () => {
    setCatModal({ mode: "add" });
  };
  const renameCategory = (cat: OtpCategory) => {
    setCatModal({ mode: "rename", cat });
  };
  const submitCategory = async (title: string) => {
    if (!catModal) return;
    try {
      if (catModal.mode === "add") {
        await invoke("otp_add_category", { title: title.trim() });
      } else {
        await invoke("otp_rename_category", { id: catModal.cat.id, title: title.trim() });
      }
      setCatModal(null);
      load();
    } catch (e) {
      alertError(t("otp.catSaveFail", { err: String(e) }));
    }
  };
  const deleteCategory = (cat: OtpCategory) => {
    setConfirmModal({
      title: t("otp.delCatTitle"),
      message: t("otp.delCatMsg", { title: cat.title }),
      danger: true,
      onConfirm: async () => {
        await invoke("otp_delete_category", { id: cat.id });
        if (activeCategory === cat.id) setActiveCategory(null);
        load();
      },
    });
  };

  const togglePin = async (token: OtpToken) => {
    await invoke("otp_toggle_pin", { id: token.id, pinned: !token.pinned });
    load();
  };

  const deleteToken = (token: OtpToken) => {
    const label = [token.issuer, token.account].filter(Boolean).join(" ") || t("otp.thisToken");
    setConfirmModal({
      title: t("otp.delTokenTitle"),
      message: t("otp.delTokenMsg", { label }),
      danger: true,
      onConfirm: async () => {
        await invoke("otp_delete", { id: token.id });
        load();
      },
    });
  };

  const saveToken = async (token: OtpToken, categoryIds: number[]) => {
    setBusy(true);
    try {
      let id = token.id;
      if (token.id) {
        await invoke("otp_update", { token });
      } else {
        id = await invoke<number>("otp_add", { token });
      }
      // 保存令牌后设置其所属分类（整体替换绑定）
      if (id) {
        await invoke("otp_set_token_categories", { tokenId: id, categoryIds });
      }
      setShowAdd(false);
      setEditing(null);
      load();
    } catch (e) {
      alertError(t("otp.saveFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const doImport = async () => {
    setBusy(true);
    try {
      const added = await invoke<OtpToken[]>("otp_import_uri", { text: importText });
      setShowImport(false);
      setImportText("");
      load();
      if (added.length === 0) theamedAlert(t("otp.noValidOtpauth"));
    } catch (e) {
      alertError(t("otp.importFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const filtered = tokens.filter((t) => {
    // 分类筛选
    if (activeCategory !== null) {
      const cat = categories.find((c) => c.id === activeCategory);
      if (cat && !cat.tokenIds.includes(t.id)) return false;
    }
    const kw = search.trim().toLowerCase();
    if (!kw) return true;
    const tagMatch = t.tags.toLowerCase().includes(kw);
    return (
      `${t.issuer} ${t.account} ${t.description}`.toLowerCase().includes(kw) || tagMatch
    );
  });

  return (
    <div className="h-full w-full flex flex-col">
      {/* 头部 */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5 flex-shrink-0">
        <KeyRound className="w-4 h-4 text-[var(--module-accent)]" />
        <span className="text-sm font-bold text-white">{t("otp.title")}</span>
        <span className="text-tiny text-slate-500">{t("otp.tokenCount", { count: tokens.length })}</span>
        <div className="flex-1" />
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("otp.searchPh")}
            className="glass-input pl-7 pr-2 py-1.5 text-body bg-black/30 border border-white/10 rounded-ctl w-40 focus:outline-none focus:border-sky-400/50"
          />
        </div>
        <button
          onClick={scanQr}
          disabled={busy}
          className="px-2.5 py-1.5 rounded-ctl text-caption bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
          title={t("otp.scanTitle")}
        >
          {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <ScanLine className="w-3 h-3" />} {t("otp.scan")}
        </button>
        <button
          onClick={() => setShowImport(true)}
          className="px-2.5 py-1.5 rounded-ctl text-caption bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
        >
          <Import className="w-3 h-3" /> {t("otp.import")}
        </button>
        <button
          onClick={() => {
            setEditing(null);
            setShowAdd(true);
          }}
          className="px-2.5 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition"
        >
          <Plus className="w-3 h-3" /> {t("otp.add")}
        </button>
      </div>

      {/* 主体：分类侧边栏 + 卡片列表 */}
      <div className="flex-1 flex min-h-0">
        {/* 分类侧边栏 */}
        <div className="w-40 flex-shrink-0 border-r border-white/5 flex flex-col overflow-y-auto">
          <div className="p-2 space-y-0.5">
            <button
              onClick={() => setActiveCategory(null)}
              className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-ctl text-caption transition cursor-pointer ${
                activeCategory === null
                  ? "bg-[var(--module-accent)]/20 text-white"
                  : "text-slate-400 hover:bg-white/5"
              }`}
            >
              <Tag className="w-3 h-3" /> {t("otp.all", { count: tokens.length })}
            </button>
            {categories.map((cat) => (
              <div
                key={cat.id}
                className={`group w-full flex items-center gap-1 px-2.5 py-1.5 rounded-ctl text-caption transition cursor-pointer ${
                  activeCategory === cat.id
                    ? "bg-[var(--module-accent)]/20 text-white"
                    : "text-slate-400 hover:bg-white/5"
                }`}
                onClick={() => setActiveCategory(cat.id)}
              >
                <Folder className="w-3 h-3 flex-shrink-0" />
                <span className="flex-1 truncate">{cat.title}</span>
                <span className="text-micro text-slate-500">{cat.tokenIds.length}</span>
                <span className="hidden group-hover:flex items-center gap-0.5">
                  <button
                    onClick={(e) => { e.stopPropagation(); renameCategory(cat); }}
                    className="p-0.5 text-slate-500 hover:text-white cursor-pointer"
                  >
                    <Pencil className="w-2.5 h-2.5" />
                  </button>
                  <button
                    onClick={(e) => { e.stopPropagation(); deleteCategory(cat); }}
                    className="p-0.5 text-slate-500 hover:text-red-400 cursor-pointer"
                  >
                    <Trash2 className="w-2.5 h-2.5" />
                  </button>
                </span>
              </div>
            ))}
            <button
              onClick={addCategory}
              className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-ctl text-caption text-slate-500 hover:text-slate-300 hover:bg-white/5 transition cursor-pointer"
            >
              <FolderPlus className="w-3 h-3" /> {t("otp.newCat")}
            </button>
          </div>
        </div>

        {/* 卡片列表 */}
        <div className="flex-1 overflow-y-auto p-4">
          {filtered.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-3">
              <KeyRound className="w-10 h-10 text-slate-600" />
              <p className="text-body">{t("otp.noTokens")}</p>
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
              {filtered.map((token) => (
                <TokenCard
                  key={token.id}
                  token={token}
                  now={now}
                  copied={copiedId === token.id}
                  categories={categories.filter((c) => c.tokenIds.includes(token.id)).map((c) => c.title)}
                  onCopy={() => copyCode(token)}
                  onTogglePin={() => togglePin(token)}
                  onDelete={() => deleteToken(token)}
                  onEdit={() => {
                    setEditing(token);
                    setShowAdd(true);
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 添加/编辑弹窗 */}
      {showAdd && (
        <TokenForm
          token={editing}
          categories={categories}
          onSave={saveToken}
          onClose={() => {
            setShowAdd(false);
            setEditing(null);
          }}
          busy={busy}
        />
      )}

      {/* 导入弹窗 */}
      {showImport && (
        <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[480px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-bold text-white">{t("otp.importTitle")}</h3>
              <button onClick={() => setShowImport(false)} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
                <X className="w-4 h-4" />
              </button>
            </div>
            <textarea
              value={importText}
              onChange={(e) => setImportText(e.target.value)}
              placeholder={t("otp.importPh")}
              className="w-full h-32 glass-input px-3 py-2 text-body font-mono bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-sky-400/50 resize-none"
            />
            <div className="flex justify-end gap-2 mt-3">
              <button onClick={() => setShowImport(false)} className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer">{t("otp.cancel")}</button>
              <button
                onClick={doImport}
                disabled={busy}
                className="px-3 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
              >
                {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Import className="w-3 h-3" />} {t("otp.import")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 新建/重命名分类弹窗 */}
      {catModal && (
        <CategoryModal
          mode={catModal.mode}
          initial={catModal.mode === "rename" ? catModal.cat.title : ""}
          busy={busy}
          onSubmit={submitCategory}
          onClose={() => setCatModal(null)}
        />
      )}

      {/* 删除确认弹窗 */}
      {confirmModal && (
        <ConfirmModal
          title={confirmModal.title}
          message={confirmModal.message}
          danger={confirmModal.danger}
          onConfirm={() => {
            const cb = confirmModal.onConfirm;
            setConfirmModal(null);
            cb();
          }}
          onClose={() => setConfirmModal(null)}
        />
      )}
    </div>
  );
}
