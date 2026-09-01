// OTP 模块面板（复刻 CloudOTP 电脑端核心：TOTP/HOTP/MOTP/Steam/Yandex + 分类 + 扫码 + 品牌图标）
import { useState, useEffect, useCallback } from "react";
import {
  Plus,
  Search,
  Copy,
  Trash2,
  Pin,
  Pencil,
  KeyRound,
  Import,
  Check,
  X,
  Loader2,
  ScanLine,
  Tag,
  Folder,
  FolderPlus,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";

interface OtpToken {
  id: number;
  issuer: string;
  account: string;
  secret: string;
  tokenType: string; // "TOTP" | "HOTP" | "MOTP" | "Steam" | "Yandex"
  algorithm: string; // "SHA1" | "SHA256" | "SHA512"
  digits: number;
  period: number;
  counter: number;
  pin: string;
  pinned: boolean;
  createdAt: number;
  description: string;
  tags: string; // 逗号分隔
  copyTimes: number;
  lastCopyTime: number;
  customIcon: string;
}

interface OtpCategory {
  id: number;
  title: string;
  createdAt: number;
  tokenIds: number[];
}

const TOKEN_TYPE_LABELS: Record<string, string> = {
  TOTP: "TOTP",
  HOTP: "HOTP",
  MOTP: "MOTP",
  Steam: "Steam",
  Yandex: "Yandex",
};

const STEAM_CHARS = "23456789BCDFGHJKMNPQRTVWXY";

// —— Base32 解码（RFC4648） ——
function base32Decode(input: string): Uint8Array {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const clean = input.toUpperCase().replace(/[\s=]/g, "");
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  for (const ch of clean) {
    const idx = alphabet.indexOf(ch);
    if (idx === -1) continue;
    acc = (acc << 5) | idx;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  return new Uint8Array(out);
}

// —— HMAC + 动态截断 ——
async function hmacDigest(algo: string, key: Uint8Array, msg: Uint8Array): Promise<Uint8Array> {
  const hashAlgo = algo === "SHA256" ? "SHA-256" : algo === "SHA512" ? "SHA-512" : "SHA-1";
  const cryptoKey = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: hashAlgo }, false, ["sign"]);
  const sig = await crypto.subtle.sign("HMAC", cryptoKey, msg);
  return new Uint8Array(sig);
}

function dynamicTruncate(digest: Uint8Array): number {
  const offset = digest[digest.length - 1] & 0x0f;
  return (
    ((digest[offset] & 0x7f) << 24) |
    ((digest[offset + 1] & 0xff) << 16) |
    ((digest[offset + 2] & 0xff) << 8) |
    (digest[offset + 3] & 0xff)
  );
}

async function hotpValue(secret: Uint8Array, counter: number, algo: string): Promise<number> {
  const msg = new Uint8Array(8);
  let c = counter;
  for (let i = 7; i >= 0; i--) {
    msg[i] = c & 0xff;
    c = Math.floor(c / 256);
  }
  const digest = await hmacDigest(algo, secret, msg);
  return dynamicTruncate(digest);
}

function padCode(code: number, digits: number): string {
  return String(code % 10 ** digits).padStart(digits, "0");
}

async function generateCode(token: OtpToken, nowMs: number): Promise<string> {
  const { tokenType, secret, algorithm, digits, period, pin } = token;
  try {
    if (tokenType === "MOTP") {
      const counter = Math.floor(nowMs / 1000 / period);
      const input = `${counter}${secret}${pin}`;
      const digest = await crypto.subtle.digest("MD5", new TextEncoder().encode(input));
      const hex = Array.from(new Uint8Array(digest)).map((b) => b.toString(16).padStart(2, "0")).join("");
      return hex.slice(0, digits);
    }
    if (tokenType === "Yandex") {
      const secretBytes = base32Decode(secret);
      // Yandex 的 key = sha256(pin + secret_bytes)，字节级拼接（非字符串拼接）
      const pinBytes = new TextEncoder().encode(pin);
      const combined = new Uint8Array(pinBytes.length + secretBytes.length);
      combined.set(pinBytes, 0);
      combined.set(secretBytes, pinBytes.length);
      const key = new Uint8Array(await crypto.subtle.digest("SHA-256", combined));
      const counter = Math.floor(nowMs / 1000 / 30);
      const msg = new Uint8Array(8);
      let c = counter;
      for (let i = 7; i >= 0; i--) { msg[i] = c & 0xff; c = Math.floor(c / 256); }
      const digest = await hmacDigest("SHA256", key, msg);
      const offset = digest[digest.length - 1] & 0x0f;
      let otp = 0n;
      for (let i = 0; i < 8; i++) otp = (otp << 8n) | BigInt(digest[offset + i]);
      otp &= 0x7fffffffffffffffn;
      let code = "";
      let v = otp;
      for (let i = 0; i < 8; i++) { code = String.fromCharCode(97 + Number(v % 26n)) + code; v /= 26n; }
      return code;
    }
    if (tokenType === "Steam") {
      const secretBytes = base32Decode(secret);
      const counter = Math.floor(nowMs / 1000 / 30);
      const msg = new Uint8Array(8);
      let c = counter;
      for (let i = 7; i >= 0; i--) { msg[i] = c & 0xff; c = Math.floor(c / 256); }
      const digest = await hmacDigest("SHA1", secretBytes, msg);
      const b = digest[19] & 0xff;
      let codePoint = ((digest[b] & 0x7f) << 24) | ((digest[b + 1] & 0xff) << 16) | ((digest[b + 2] & 0xff) << 8) | (digest[b + 3] & 0xff);
      let code = "";
      for (let i = 0; i < 5; i++) {
        code += STEAM_CHARS[codePoint % STEAM_CHARS.length];
        codePoint = Math.floor(codePoint / STEAM_CHARS.length);
      }
      return code;
    }
    // TOTP / HOTP
    const secretBytes = base32Decode(secret);
    const counter = tokenType === "HOTP" ? token.counter : Math.floor(nowMs / 1000 / period);
    const value = await hotpValue(secretBytes, counter, algorithm);
    return padCode(value, digits);
  } catch {
    return "ERROR";
  }
}

function remainingSeconds(token: OtpToken, nowMs: number): number {
  if (token.tokenType === "HOTP") return 0;
  const period = token.period || 30;
  return period - (Math.floor(nowMs / 1000) % period);
}

// 分组显示验证码：如 "123456" -> "123 456"
function formatCode(code: string): string {
  if (code.length <= 4) return code;
  const mid = Math.ceil(code.length / 2);
  return code.slice(0, mid) + " " + code.slice(mid);
}

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
      await navigator.clipboard.writeText(code);
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
          alert(t("otp.scanNotOtpauth"));
        }
      } else {
        alert(t("otp.scanIgnored", { text }));
      }
    } catch (e) {
      alert(t("otp.scanFail", { err: String(e) }));
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
      alert(t("otp.catSaveFail", { err: String(e) }));
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
      alert(t("otp.saveFail", { err: String(e) }));
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
      if (added.length === 0) alert(t("otp.noValidOtpauth"));
    } catch (e) {
      alert(t("otp.importFail", { err: String(e) }));
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
        <span className="text-[10px] text-slate-500">{t("otp.tokenCount", { count: tokens.length })}</span>
        <div className="flex-1" />
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500 pointer-events-none" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("otp.searchPh")}
            className="glass-input pl-7 pr-2 py-1.5 text-xs bg-black/30 border border-white/10 rounded-lg w-40 focus:outline-none focus:border-sky-400/50"
          />
        </div>
        <button
          onClick={scanQr}
          disabled={busy}
          className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
          title={t("otp.scanTitle")}
        >
          {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <ScanLine className="w-3 h-3" />} {t("otp.scan")}
        </button>
        <button
          onClick={() => setShowImport(true)}
          className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
        >
          <Import className="w-3 h-3" /> {t("otp.import")}
        </button>
        <button
          onClick={() => {
            setEditing(null);
            setShowAdd(true);
          }}
          className="px-2.5 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition"
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
              className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[11px] transition cursor-pointer ${
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
                className={`group w-full flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] transition cursor-pointer ${
                  activeCategory === cat.id
                    ? "bg-[var(--module-accent)]/20 text-white"
                    : "text-slate-400 hover:bg-white/5"
                }`}
                onClick={() => setActiveCategory(cat.id)}
              >
                <Folder className="w-3 h-3 flex-shrink-0" />
                <span className="flex-1 truncate">{cat.title}</span>
                <span className="text-[9px] text-slate-500">{cat.tokenIds.length}</span>
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
              className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[11px] text-slate-500 hover:text-slate-300 hover:bg-white/5 transition cursor-pointer"
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
              <p className="text-xs">{t("otp.noTokens")}</p>
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
          <div className="w-[480px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5" onClick={(e) => e.stopPropagation()}>
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
              className="w-full h-32 glass-input px-3 py-2 text-xs font-mono bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-sky-400/50 resize-none"
            />
            <div className="flex justify-end gap-2 mt-3">
              <button onClick={() => setShowImport(false)} className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer">{t("otp.cancel")}</button>
              <button
                onClick={doImport}
                disabled={busy}
                className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
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

// 品牌图标：根据 customIcon / issuer 显示品牌标识（字母 + 主题色块）。
// 电脑端不内嵌 logo 图片资源，用「品牌名首字母 + 哈希色」作为图标，简洁且无需资源。
function BrandIcon({ token }: { token: OtpToken }) {
  const label = token.customIcon || token.issuer || token.account || "?";
  const initial = (label.trim()[0] || "?").toUpperCase();
  // 根据 issuer 生成稳定的哈希色
  let hash = 0;
  const src = token.issuer || token.account;
  for (let i = 0; i < src.length; i++) hash = (hash * 31 + src.charCodeAt(i)) | 0;
  const hue = Math.abs(hash) % 360;
  return (
    <div
      className="w-8 h-8 rounded-lg flex items-center justify-center text-[13px] font-bold text-white flex-shrink-0"
      style={{ background: `linear-gradient(135deg, hsl(${hue},65%,45%), hsl(${(hue + 40) % 360},65%,35%))` }}
    >
      {initial}
    </div>
  );
}

// 令牌卡片
function TokenCard({
  token,
  now,

  copied,
  categories,
  onCopy,
  onTogglePin,
  onDelete,
  onEdit,
}: {
  token: OtpToken;
  now: number;
  copied: boolean;
  categories: string[];
  onCopy: () => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onEdit: () => void;
}) {
  const { t } = useTranslation();
  const [code, setCode] = useState("------");
  const [remain, setRemain] = useState(30);

  useEffect(() => {
    let alive = true;
    generateCode(token, now).then((c) => {
      if (alive) setCode(c);
    });
    setRemain(remainingSeconds(token, now));
    return () => {
      alive = false;
    };
  }, [token, now]);

  const isHotp = token.tokenType === "HOTP";
  const progress = isHotp ? 0 : remain / (token.period || 30);

  return (
    <div
      className={`group relative rounded-xl border p-3 pb-9 transition cursor-pointer ${
        token.pinned
          ? "bg-amber-500/[0.06] border-amber-500/25"
          : "bg-white/[0.03] border-white/10 hover:bg-white/[0.05] hover:border-white/20"
      }`}
      onClick={onCopy}
      title={t("otp.copyCode")}
    >
      <div className="flex items-start gap-2">
        <BrandIcon token={token} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[13px] font-semibold text-white truncate">
              {token.issuer || t("otp.unknown")}
            </span>
            {token.pinned && <Pin className="w-3 h-3 text-amber-400 flex-shrink-0" />}
          </div>
          <div className="text-[10px] text-slate-500 truncate">{token.account}</div>
          {token.description && (
            <div className="text-[9px] text-slate-600 truncate mt-0.5">{token.description}</div>
          )}
          {/* 所属分类 */}
          {categories.length > 0 && (
            <div className="flex flex-wrap gap-1 mt-1">
              {categories.map((c, i) => (
                <span
                  key={i}
                  className="inline-flex items-center gap-0.5 text-[8px] px-1 py-0.5 rounded bg-[var(--module-accent)]/10 text-[var(--module-accent)] border border-[var(--module-accent)]/20"
                >
                  <Folder className="w-2 h-2" />{c}
                </span>
              ))}
            </div>
          )}
          {/* 标签 */}
          {token.tags && (
            <div className="flex flex-wrap gap-1 mt-1">
              {token.tags.split(",").filter(Boolean).map((t, i) => (
                <span key={i} className="text-[8px] px-1 py-0.5 rounded bg-white/5 text-slate-400">
                  #{t.trim()}
                </span>
              ))}
            </div>
          )}
        </div>
        <span className="text-[9px] px-1.5 py-0.5 rounded bg-white/5 text-slate-400 font-mono flex-shrink-0">
          {TOKEN_TYPE_LABELS[token.tokenType] || token.tokenType}
        </span>
      </div>

      <div className="mt-3 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xl font-mono font-bold text-white tracking-wider select-none">
            {formatCode(code)}
          </span>
          {copied ? (
            <Check className="w-4 h-4 text-emerald-400" />
          ) : (
            <Copy className="w-3.5 h-3.5 text-slate-500" />
          )}
        </div>
        {/* 环形倒计时 */}
        {!isHotp && (
          <svg width="24" height="24" viewBox="0 0 24 24" className="flex-shrink-0">
            <circle cx="12" cy="12" r="9" fill="none" stroke="rgba(255,255,255,0.1)" strokeWidth="2.5" />
            <circle
              cx="12"
              cy="12"
              r="9"
              fill="none"
              stroke="var(--module-accent)"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeDasharray={`${2 * Math.PI * 9}`}
              strokeDashoffset={`${2 * Math.PI * 9 * (1 - progress)}`}
              transform="rotate(-90 12 12)"
              style={{ transition: "stroke-dashoffset 1s linear" }}
            />
            <text x="12" y="15" textAnchor="middle" className="fill-slate-400" style={{ fontSize: 8 }}>
              {remain}
            </text>
          </svg>
        )}
      </div>

      {/* hover 操作（左下角，避免与顶部右侧的类型/标签重叠） */}
      <div className="absolute left-2 bottom-2 hidden group-hover:flex items-center gap-1">
        <button
          onClick={(e) => { e.stopPropagation(); onEdit(); }}
          className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
          title={t("otp.edit")}
        >
          <Pencil className="w-3 h-3" />
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); onTogglePin(); }}
          className={`p-1 rounded hover:bg-white/10 cursor-pointer ${token.pinned ? "text-amber-400" : "text-slate-400 hover:text-amber-300"}`}
          title={token.pinned ? t("otp.unpin") : t("otp.pin")}
        >
          <Pin className="w-3 h-3" />
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); onDelete(); }}
          className="p-1 rounded hover:bg-red-500/15 text-slate-400 hover:text-red-400 cursor-pointer"
          title={t("otp.delete")}
        >
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
    </div>
  );
}

// 添加/编辑表单
function TokenForm({
  token,
  categories,
  onSave,
  onClose,
  busy,
}: {
  token: OtpToken | null;
  categories: OtpCategory[];
  onSave: (t: OtpToken, categoryIds: number[]) => void;
  onClose: () => void;
  busy: boolean;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState<OtpToken>(
    token || {
      id: 0,
      issuer: "",
      account: "",
      secret: "",
      tokenType: "TOTP",
      algorithm: "SHA1",
      digits: 6,
      period: 30,
      counter: 0,
      pin: "",
      pinned: false,
      createdAt: 0,
      description: "",
      tags: "",
      copyTimes: 0,
      lastCopyTime: 0,
      customIcon: "",
    }
  );

  const set = (k: keyof OtpToken, v: unknown) => setForm((f) => ({ ...f, [k]: v }));

  // 该令牌当前所属分类（编辑时回填；新增为空）
  const [selectedCatIds, setSelectedCatIds] = useState<number[]>(() =>
    token
      ? categories.filter((c) => c.tokenIds.includes(token.id)).map((c) => c.id)
      : []
  );

  const toggleCat = (id: number) =>
    setSelectedCatIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]
    );

  const submit = () => {
    if (!form.secret.trim()) {
      alert(t("otp.needSecret"));
      return;
    }
    onSave(form, selectedCatIds);
  };

  const isSteam = form.tokenType === "Steam";
  const isYandex = form.tokenType === "Yandex";
  const isMotp = form.tokenType === "MOTP";
  const isHotp = form.tokenType === "HOTP";

  return (
    <div className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div className="w-[460px] max-w-[95vw] max-h-[90vh] overflow-y-auto rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-bold text-white">{token ? t("otp.editToken") : t("otp.addToken")}</h3>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10 text-slate-400 cursor-pointer">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="space-y-3">
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.type")}</label>
            <select
              value={form.tokenType}
              onChange={(e) => {
                const t = e.target.value;
                setForm((f) => ({
                  ...f,
                  tokenType: t,
                  digits: t === "Steam" ? 5 : t === "Yandex" ? 8 : 6,
                  algorithm: t === "Steam" || t === "Yandex" || t === "MOTP" ? "SHA1" : f.algorithm,
                }));
              }}
              className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
            >
              <option value="TOTP">{t("otp.totp")}</option>
              <option value="HOTP">{t("otp.hotp")}</option>
              <option value="MOTP">{t("otp.motp")}</option>
              <option value="Steam">Steam</option>
              <option value="Yandex">Yandex</option>
            </select>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.issuer")}</label>
              <input
                value={form.issuer}
                onChange={(e) => set("issuer", e.target.value)}
                placeholder={t("otp.issuerPh")}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.account")}</label>
              <input
                value={form.account}
                onChange={(e) => set("account", e.target.value)}
                placeholder={t("otp.accountPh")}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
          </div>

          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.secret")}</label>
            <input
              value={form.secret}
              onChange={(e) => set("secret", e.target.value)}
              placeholder={t("otp.secretPh")}
              className="w-full glass-input px-3 py-2 text-xs font-mono bg-black/30 border border-white/10 rounded-lg focus:outline-none"
            />
          </div>

          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.desc")}</label>
            <input
              value={form.description}
              onChange={(e) => set("description", e.target.value)}
              placeholder={t("otp.descPh")}
              className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
            />
          </div>

          {/* 分类多选 */}
          <div>
            <label className="text-[10px] text-slate-400 mb-1 block">
              {t("otp.category", { count: selectedCatIds.length })} {selectedCatIds.length > 0 && <span className="text-slate-500">({t("otp.selected", { count: selectedCatIds.length })})</span>}
            </label>
            {categories.length === 0 ? (
              <p className="text-[10px] text-slate-500">{t("otp.categoryHint")}</p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {categories.map((c) => {
                  const on = selectedCatIds.includes(c.id);
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => toggleCat(c.id)}
                      className={`vex-chip px-2.5 py-1 rounded-full text-[10px] border transition-colors cursor-pointer ${
                        on
                          ? "vex-chip-active bg-[var(--module-accent)]/20 border-[var(--module-accent)] text-[var(--module-accent)]"
                          : "bg-white/[0.03] border-white/10 text-slate-400 hover:border-white/25"
                      }`}
                    >
                      {on ? "✓ " : ""}{c.title}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.tags")}</label>
              <input
                value={form.tags}
                onChange={(e) => set("tags", e.target.value)}
                placeholder={t("otp.tagsPh")}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.iconChar")}</label>
              <input
                value={form.customIcon}
                onChange={(e) => set("customIcon", e.target.value)}
                placeholder={t("otp.iconPh")}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
          </div>

          {!isSteam && !isYandex && !isMotp && (
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.algorithm")}</label>
                <select
                  value={form.algorithm}
                  onChange={(e) => set("algorithm", e.target.value)}
                  className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
                >
                  <option value="SHA1">SHA1</option>
                  <option value="SHA256">SHA256</option>
                  <option value="SHA512">SHA512</option>
                </select>
              </div>
              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.digits")}</label>
                <select
                  value={form.digits}
                  onChange={(e) => set("digits", Number(e.target.value))}
                  className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
                >
                  {[5, 6, 7, 8].map((d) => (
                    <option key={d} value={d}>{t("otp.digitsCount", { count: d })}</option>
                  ))}
                </select>
              </div>
            </div>
          )}

          {isHotp ? (
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.counter")}</label>
              <input
                type="number"
                value={form.counter}
                onChange={(e) => set("counter", Number(e.target.value))}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
          ) : !isSteam && !isYandex ? (
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">{t("otp.period")}</label>
              <input
                type="number"
                value={form.period}
                onChange={(e) => set("period", Number(e.target.value))}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
          ) : null}

          {(isMotp || isYandex) && (
            <div>
              <label className="text-[10px] text-slate-400 mb-1 block">PIN</label>
              <input
                value={form.pin}
                onChange={(e) => set("pin", e.target.value)}
                placeholder={isMotp ? t("otp.pinPh") : t("otp.pinPhShort")}
                className="w-full glass-input px-3 py-2 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none"
              />
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 mt-4">
          <button onClick={onClose} className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer">{t("otp.cancel")}</button>
          <button
            onClick={submit}
            disabled={busy}
            className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("otp.save")}
          </button>
        </div>
      </div>
    </div>
  );
}

// 新建 / 重命名分类弹窗（替代原生 prompt，美化样式）
function CategoryModal({
  mode,
  initial,
  busy,
  onSubmit,
  onClose,
}: {
  mode: "add" | "rename";
  initial: string;
  busy: boolean;
  onSubmit: (title: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(initial);

  const submit = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    onSubmit(trimmed);
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") submit();
  };

  return (
    <div
      className="fixed inset-0 z-[120] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
    >
      <div
        className="w-[360px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 mb-4">
          <div className="w-9 h-9 rounded-xl bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
            {mode === "add" ? (
              <FolderPlus className="w-4 h-4 text-[var(--module-accent)]" />
            ) : (
              <Pencil className="w-4 h-4 text-[var(--module-accent)]" />
            )}
          </div>
          <div className="flex-1">
            <h3 className="text-sm font-bold text-white">
              {mode === "add" ? t("otp.newCatTitle") : t("otp.renameCatTitle")}
            </h3>
            <p className="text-[10px] text-slate-500">
              {mode === "add" ? t("otp.catHintAdd") : t("otp.catHintRename")}
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={onKey}
          placeholder={t("otp.catNamePh")}
          className="w-full glass-input px-3 py-2.5 text-xs bg-black/30 border border-white/10 rounded-lg focus:outline-none focus:border-[var(--module-accent)]/60 placeholder:text-slate-600"
        />

        <div className="flex justify-end gap-2 mt-4">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer"
          >
            {t("otp.cancel")}
          </button>
          <button
            onClick={submit}
            disabled={busy || !name.trim()}
            className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />} {t("otp.save")}
          </button>
        </div>
      </div>
    </div>
  );
}

// 通用删除确认弹窗（替代原生 confirm，主题色随模块动态变化）
function ConfirmModal({
  title,
  message,
  danger,
  onConfirm,
  onClose,
}: {
  title: string;
  message: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
    >
      <div
        className="w-[360px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 mb-4">
          <div
            className={`w-9 h-9 rounded-xl flex items-center justify-center border ${
              danger
                ? "bg-rose-500/15 border-rose-500/30 text-rose-400"
                : "bg-[var(--module-accent)]/15 border-[var(--module-accent)]/30 text-[var(--module-accent)]"
            }`}
          >
            <Trash2 className="w-4 h-4" />
          </div>
          <div className="flex-1">
            <h3 className="text-sm font-bold text-white">{title}</h3>
            <p className="text-[10px] text-slate-500">
              {danger ? t("otp.confirmDanger") : t("otp.confirmNormal")}
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <p className="text-xs text-slate-300 leading-relaxed mb-5">{message}</p>

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer"
          >
            {t("otp.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className={`px-4 py-1.5 rounded-lg text-[11px] text-white font-semibold cursor-pointer hover:opacity-85 flex items-center gap-1 ${
              danger
                ? "bg-rose-600 hover:bg-rose-500"
                : "bg-[var(--module-accent)] hover:opacity-85"
            }`}
          >
            <Trash2 className="w-3 h-3" /> {t("otp.delete")}
          </button>
        </div>
      </div>
    </div>
  );
}
