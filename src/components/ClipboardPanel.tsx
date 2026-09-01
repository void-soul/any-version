// 剪贴板历史面板（全宽紧凑列表 + 单行截断 + 固定行高虚拟滚动 + 触底加载）
import { useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import VexEmptyState from "./VexEmptyState";
import {
  Clipboard,
  Search,
  FileText,
  Pin,
  Copy,
  ClipboardPaste,
  Trash2,
  Settings2,
  X,
  Eraser,
  Plus,
  Clock,
  AppWindow,
  Check,
  Loader2,
  Image as ImageIcon,
  ZoomIn,

} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "./shared/Toast";
import { SharedModal } from "./shared/Modal";
import { SharedButton } from "./shared/Button";

interface ClipboardItem {
  id: number;
  kind: string; // "text" | "image"
  content: string | null;
  imagePath: string | null;
  thumbPath: string | null;
  width: number;
  height: number;
  sourceApp: string;
  pinned: boolean;
  createdAt: number;
  formats: string[]; // 复制时的剪贴板格式（CopyQ 式）
}

interface ClipboardSettings {
  enabled: boolean;
  maxItems: number;
  storeImages: boolean;
  ignoreBlank: boolean;
  ignoreShort: boolean;
}

const PAGE_SIZE = 100;
// 固定行高（虚拟滚动依赖）
const ROW_H = 52;
const OVERSCAN = 8;

// 缩略图 data-url 缓存（虚拟滚动频繁进出视口时避免重复请求）
const imageCache = new Map<number, string>();

function fmtTime(ts: number, t: (key: string, opts?: Record<string, unknown>) => string): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.toDateString() === now.toDateString()) return hm;
  const yest = new Date(now);
  yest.setDate(now.getDate() - 1);
  if (d.toDateString() === yest.toDateString()) return t("clip.yesterday", { hm });
  return t("clip.date", { m: d.getMonth() + 1, d: d.getDate(), hm });
}

// 图片缩略图（带全局缓存）
function ImageThumb({ item }: { item: ClipboardItem }) {
  const [src, setSrc] = useState<string>(imageCache.get(item.id) || "");
  useEffect(() => {
    if (src) return;
    let alive = true;
    invoke<string>("clipboard_get_image", { id: item.id, thumb: true })
      .then((d) => {
        imageCache.set(item.id, d);
        if (alive) setSrc(d);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [item.id]);
  if (!src) {
    return (
      <div className="w-[56px] h-9 rounded-md bg-white/5 border border-white/10 flex items-center justify-center flex-shrink-0">
        <Loader2 className="w-3.5 h-3.5 text-slate-500 animate-spin" />
      </div>
    );
  }
  return (
    <img
      src={src}
      alt="clipboard"
      className="w-[56px] h-9 rounded-md object-cover border border-white/10 bg-black/40 flex-shrink-0"
      draggable={false}
    />
  );
}

// 图片原图预览（thumb=false 加载原图；点击图片切换「适应窗口 / 实际大小」）
function PreviewImage({ item }: { item: ClipboardItem }) {
  const { t } = useTranslation();
  const [src, setSrc] = useState("");
  const [zoom, setZoom] = useState(false);
  useEffect(() => {
    let alive = true;
    setSrc("");
    setZoom(false);
    invoke<string>("clipboard_get_image", { id: item.id, thumb: false })
      .then((d) => alive && setSrc(d))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [item.id]);
  if (!src) {
    return (
      <div className="flex-1 min-h-[220px] flex items-center justify-center bg-black/25">
        <Loader2 className="w-6 h-6 text-slate-500 animate-spin" />
      </div>
    );
  }
  return (
    <div
      className={`flex-1 min-h-0 overflow-auto bg-black/25 ${zoom ? "p-4" : "flex items-center justify-center p-4"}`}
      onClick={() => setZoom((z) => !z)}
      title={zoom ? t("clip.zoomFitTip") : t("clip.zoomActualTip")}
    >
      <img
        src={src}
        alt={t("clip.altPreview")}
        draggable={false}
        className={zoom ? "cursor-zoom-out rounded-md" : "max-w-full max-h-[58vh] object-contain cursor-zoom-in rounded-md"}
      />
    </div>
  );
}

// 预览弹窗：文本显示完整内容，图片显示原图
function PreviewModal({
  item,
  onClose,
  onCopy,
  onPaste,
}: {
  item: ClipboardItem;
  onClose: () => void;
  onCopy: () => void;
  onPaste: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
    >
      <div
        className={`flex flex-col rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl shadow-black/60 overflow-hidden max-h-[90vh] ${
          item.kind === "image" ? "w-[820px] max-w-[95vw]" : "w-[640px] max-w-[95vw]"
        }`}
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部：类型 + 来源 + 时间 + 格式 */}
        <div className="flex items-center justify-between gap-3 px-4 py-3 border-b border-white/10 bg-white/[0.02] flex-shrink-0">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="p-1.5 rounded-lg bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] flex-shrink-0">
              {item.kind === "image" ? <ImageIcon className="w-4 h-4" /> : <FileText className="w-4 h-4" />}
            </div>
            <div className="min-w-0">
              <h4 className="text-[13px] font-bold text-white">{item.kind === "image" ? t("clip.imagePreview") : t("clip.textPreview")}</h4>
              <p className="text-[10px] text-slate-500 truncate flex items-center gap-2 mt-0.5">
                <span className="inline-flex items-center gap-1 min-w-0">
                  <AppWindow className="w-2.5 h-2.5 flex-shrink-0" />
                  <span className="truncate">{item.sourceApp || t("clip.unknownSource")}</span>
                </span>
                <span className="inline-flex items-center gap-1 flex-shrink-0">
                  <Clock className="w-2.5 h-2.5" />
                  {fmtTime(item.createdAt, t)}
                </span>
              </p>
            </div>
          </div>
          {item.formats && item.formats.length > 0 && (
            <span
              className="hidden lg:inline text-[9px] text-slate-500 truncate max-w-[240px] flex-shrink-0"
              title={item.formats.join("\n")}
            >
              {item.formats.join(" · ")}
            </span>
          )}
          <button
            onClick={onClose}
            className="p-1.5 rounded-md hover:bg-white/10 text-slate-500 hover:text-white transition-all cursor-pointer flex-shrink-0"
            title={t("clip.closeEsc")}
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* 内容区 */}
        {item.kind === "text" ? (
          <div className="p-4 overflow-y-auto flex-1 min-h-0 bg-black/20">
            <pre className="whitespace-pre-wrap break-words font-mono text-[12px] leading-relaxed text-slate-200 select-text">
              {item.content || t("clip.emptyContent")}
            </pre>
          </div>
        ) : (
          <PreviewImage item={item} />
        )}

        {/* 底部操作 */}
        <div className="flex items-center justify-between gap-3 px-4 py-2.5 border-t border-white/10 bg-white/[0.02] flex-shrink-0">
          <div className="text-[10px] text-slate-500 flex items-center gap-3 min-w-0">
            {item.kind === "image" ? (
              <>
                {item.width > 0 && item.height > 0 && <span>{item.width}×{item.height}</span>}
                <span className="hidden sm:inline-flex items-center gap-1">
                  <ZoomIn className="w-3 h-3" /> {t("clip.zoomBtn")}
                </span>
              </>
            ) : (
              item.content && <span>{t("clip.charsCount", { n: item.content.length })}</span>
            )}
          </div>
          <div className="flex items-center gap-2 flex-shrink-0">
            <button
              onClick={onCopy}
              className="px-3 h-8 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 text-[11px] text-slate-300 transition-all cursor-pointer flex items-center gap-1.5"
              title={t("clip.copyTip")}
            >
              <Copy className="w-3 h-3" /> {t("clip.copy")}
            </button>
            <button
              onClick={onPaste}
              className="px-3 h-8 rounded-lg bg-[var(--module-accent)] hover:opacity-85 text-[11px] font-semibold text-white transition-all cursor-pointer flex items-center gap-1.5"
              title={t("clip.copyPasteTip")}
            >
              <ClipboardPaste className="w-3 h-3" /> {t("clip.copyPaste")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// 单行紧凑行组件（固定高度由外层 ROW_H 保证）
function Row({
  item,
  copied,
  onPreview,
  onCopy,
  onPaste,
  onPin,
  onDelete,
}: {
  item: ClipboardItem;
  copied: boolean;
  onPreview: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onPin: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      onClick={onPaste}
      className={`h-full flex items-center gap-3 rounded-lg border px-2.5 transition-all cursor-pointer group ${
        item.pinned
          ? "bg-amber-500/[0.06] border-amber-500/25"
          : "bg-white/[0.03] border-white/10 hover:bg-white/[0.06] hover:border-white/20"
      }`}
      title={t("clip.rowTip")}
    >
      {/* 类型图标 / 缩略图 */}
      {item.kind === "image" ? (
        <ImageThumb item={item} />
      ) : (
        <div className="w-[56px] h-9 rounded-md bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] flex items-center justify-center flex-shrink-0">
          <FileText className="w-4 h-4 text-[var(--module-accent)]" />
        </div>
      )}

      {/* 内容：单行截断（悬停显示完整）+ 复制时的剪贴板格式 */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 min-w-0">
          <p
            className="text-[12px] text-slate-200 truncate font-mono"
            title={item.kind === "text" ? item.content || undefined : undefined}
          >
            {item.kind === "text" ? item.content : item.width && item.height ? t("clip.imageSize", { w: item.width, h: item.height }) : t("clip.imageContent")}
          </p>
          {item.formats && item.formats.length > 0 && (
            <span
              className="text-[9px] text-slate-500 whitespace-nowrap truncate max-w-[200px] flex-shrink-0 hidden xl:inline"
              title={item.formats.join("\n")}
            >
              {item.formats.join(" · ")}
            </span>
          )}
        </div>
      </div>

      {/* 元信息 */}
      <div className="flex items-center gap-2 text-[10px] text-slate-500 whitespace-nowrap flex-shrink-0 hidden md:flex">
        <span className="inline-flex items-center gap-1">
          <AppWindow className="w-3 h-3" />
          {item.sourceApp || t("clip.unknownSource")}
        </span>
        <span className="inline-flex items-center gap-1">
          <Clock className="w-3 h-3" />
          {fmtTime(item.createdAt, t)}
        </span>
      </div>

      {/* 操作 */}
      <div className="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity">
        {item.pinned && <Pin className="w-3 h-3 text-amber-400" />}
        <button
          onClick={(e) => {
            e.stopPropagation();
            onPreview();
          }}
          className="p-1.5 rounded-md hover:bg-white/10 text-slate-400 hover:text-sky-300 transition-all cursor-pointer"
          title={t("clip.previewContent")}
        >
          <ZoomIn className="w-3.5 h-3.5" />
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onCopy();
          }}
          className="p-1.5 rounded-md hover:bg-white/10 text-slate-400 hover:text-sky-300 transition-all cursor-pointer"
          title={t("clip.copyTip")}
        >
          {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onPaste();
          }}
          className="p-1.5 rounded-md hover:bg-white/10 text-slate-400 hover:text-[var(--module-accent)] transition-all cursor-pointer"
          title={t("clip.copyPasteTip")}
        >
          <ClipboardPaste className="w-3.5 h-3.5" />
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onPin();
          }}
          className={`p-1.5 rounded-md hover:bg-white/10 transition-all cursor-pointer ${
            item.pinned ? "text-amber-400" : "text-slate-400 hover:text-amber-300"
          }`}
          title={item.pinned ? t("clip.unpinTip") : t("clip.pinTip")}
        >
          <Pin className="w-3.5 h-3.5" />
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
          className="p-1.5 rounded-md hover:bg-red-500/15 text-slate-400 hover:text-red-400 transition-all cursor-pointer"
          title={t("common.delete")}
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  );
}

export default function ClipboardPanel() {
  const { t } = useTranslation();
  const [items, setItems] = useState<ClipboardItem[]>([]);
  const [total, setTotal] = useState(0);
  const [keyword, setKeyword] = useState("");
  // 输入框的临时值：输入过程只更新这里，不触发搜索；点击「搜索」按钮或回车后才同步到 keyword。
  const [searchInput, setSearchInput] = useState("");
  const [kind, setKind] = useState(""); // "" | "text" | "image"
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadedAll, setLoadedAll] = useState(false);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewH, setViewH] = useState(600);

  const [settings, setSettings] = useState<ClipboardSettings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [ignoredApps, setIgnoredApps] = useState<string[]>([]);
  const [newApp, setNewApp] = useState("");
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [_busy, setBusy] = useState("");
  // 复制/粘贴结果提示（管理员模式下 alert 可能被 UAC 遮挡，用应用内 toast 确保可见）
  const showToast = useCallback((kind: "ok" | "err", msg: string) => toast(msg, kind), []);

  // 预览：通过行内「预览」按钮打开弹窗
  const [preview, setPreview] = useState<ClipboardItem | null>(null);

  const itemsRef = useRef<ClipboardItem[]>([]);
  itemsRef.current = items;
  const listRef = useRef<HTMLDivElement>(null);

  const load = useCallback(
    async (reset: boolean) => {
      const offset = reset ? 0 : itemsRef.current.length;
      setLoading(true);
      try {
        const res = await invoke<{ items: ClipboardItem[]; total: number }>("clipboard_get_items", {
          keyword,
          kind,
          pinnedOnly,
          limit: PAGE_SIZE,
          offset,
        });
        setItems(reset ? res.items : [...itemsRef.current, ...res.items]);
        setTotal(res.total);
        setLoadedAll(offset + res.items.length >= res.total);
      } catch (e) {
        console.error("加载剪贴板历史失败", e);
      } finally {
        setLoading(false);
      }
    },
    [keyword, kind, pinnedOnly]
  );

  // 首屏 + 条件变化（reset 并回滚到顶部）
  useEffect(() => {
    listRef.current?.scrollTo({ top: 0 });
    setScrollTop(0);
    load(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keyword, kind, pinnedOnly]);

  // 挂载后同步一次可视区高度（虚拟滚动窗口基准）
  useEffect(() => {
    const el = listRef.current;
    if (el) setViewH(el.clientHeight);
  }, []);

  // 监听后端「剪贴板有新内容」事件
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen("clipboard-updated", () => load(true)).then((fn) => (unlisten = fn));
    return () => {
      if (unlisten) unlisten();
    };
  }, [load]);

  // 输入框值变化：只更新临时输入，不触发搜索（避免每次按键/停顿都查询导致卡顿）。
  const onSearchChange = (v: string) => {
    setSearchInput(v);
  };

  // 点击「搜索」按钮或回车时触发：把临时输入同步到 keyword，由 useEffect 驱动重新查询。
  const handleSearch = () => {
    setKeyword(searchInput);
  };

  // 虚拟滚动 onScroll：更新可视窗口 + 触底自动加载
  const onListScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setScrollTop(el.scrollTop);
    setViewH(el.clientHeight);
    if (!loading && !loadedAll && el.scrollHeight - el.scrollTop - el.clientHeight < 300) {
      load(false);
    }
  };

  const openPreview = (item: ClipboardItem) => {
    setPreview(item);
  };

  const closePreview = () => {
    setPreview(null);
  };

  const refreshSettings = useCallback(async () => {
    try {
      setSettings(await invoke<ClipboardSettings>("clipboard_get_settings"));
      setIgnoredApps(await invoke<string[]>("clipboard_get_ignored_apps"));
    } catch (e) {
      console.error("加载剪贴板设置失败", e);
    }
  }, []);

  useEffect(() => {
    refreshSettings();
  }, [refreshSettings]);

  const copyItem = async (id: number) => {
    try {
      await invoke("clipboard_copy_item", { id });
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1200);
      showToast("ok", t("clip.copiedToast"));
    } catch (e) {
      const msg = t("clip.copyFail", { err: String(e) });
      showToast("err", msg);
      console.error(msg);
    }
  };

  const pasteItem = async (id: number) => {
    setBusy("paste");
    try {
      await invoke("clipboard_paste_item", { id });
      setPreview(null);
      showToast("ok", t("clip.pastedToast"));
    } catch (e) {
      const msg = t("clip.pasteFail", { err: String(e) });
      showToast("err", msg);
      console.error(msg);
    } finally {
      setBusy("");
    }
  };

  const togglePin = async (item: ClipboardItem) => {
    try {
      await invoke("clipboard_pin_item", { id: item.id, pinned: !item.pinned });
      setItems((prev) => prev.map((i) => (i.id === item.id ? { ...i, pinned: !item.pinned } : i)));
      load(true);
    } catch (e) {
      alert(String(e));
    }
  };

  const deleteItem = async (id: number) => {
    try {
      await invoke("clipboard_delete_item", { id });
      setItems((prev) => prev.filter((i) => i.id !== id));
      setTotal((t) => Math.max(0, t - 1));
    } catch (e) {
      alert(String(e));
    }
  };

  const clearHistory = async () => {
    if (!window.confirm(t("clip.clearConfirm"))) return;
    try {
      await invoke("clipboard_clear_history", { keepPinned: true });
      load(true);
    } catch (e) {
      alert(String(e));
    }
  };

  const saveSettings = async () => {
    if (!settings) return;
    try {
      await invoke("clipboard_save_settings", { settings });
      setShowSettings(false);
    } catch (e) {
      alert(String(e));
    }
  };

  const addIgnoredApp = async () => {
    const app = newApp.trim();
    if (!app) return;
    try {
      await invoke("clipboard_add_ignored_app", { app });
      setIgnoredApps((prev) => [...prev, app]);
      setNewApp("");
    } catch (e) {
      alert(String(e));
    }
  };

  const removeIgnoredApp = async (app: string) => {
    try {
      await invoke("clipboard_remove_ignored_app", { app });
      setIgnoredApps((prev) => prev.filter((a) => a !== app));
    } catch (e) {
      alert(String(e));
    }
  };

  const hasMore = !loadedAll && items.length > 0;
  const totalRows = items.length + (hasMore ? 1 : 0);
  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(totalRows, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN);

  return (
    <div className="px-4 py-3 w-full space-y-3 select-none text-slate-200 h-full flex flex-col min-h-0">
      {/* 头部 */}
      <div className="flex items-center justify-between gap-4 shrink-0">
        <div className="flex items-center gap-2.5 min-w-0">
          <div className="p-2 rounded-xl bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] flex-shrink-0">
            <Clipboard className="w-5 h-5" />
          </div>
          <div className="min-w-0">
            <h3 className="text-[14px] font-bold text-white flex items-center gap-2">
              {t("clip.title")}
              <span
                className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-medium border ${
                  settings?.enabled
                    ? "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border-[var(--module-accent-ring)]"
                    : "bg-white/5 text-slate-400 border-white/10"
                }`}
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full ${settings?.enabled ? "bg-[var(--module-accent)] animate-pulse" : "bg-slate-500"}`}
                />
                {settings?.enabled ? t("clip.monitoring") : t("clip.paused")}
              </span>
            </h3>
            <p className="text-[10px] text-slate-400 mt-0.5 truncate">
              {t("clip.summary", { n: total })}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          <button
            onClick={clearHistory}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 hover:bg-red-500/15 border border-white/10 text-[10px] text-slate-300 hover:text-red-300 transition-all cursor-pointer flex items-center gap-1.5"            title={t("clip.clearTip")}
            >
            <Eraser className="w-3 h-3" /> {t("clip.clear")}
          </button>
          <button
            onClick={() => setShowSettings(true)}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 text-[10px] text-slate-300 transition-all cursor-pointer flex items-center gap-1.5"
            title={t("clip.settingsTip")}
          >
            <Settings2 className="w-3 h-3" /> {t("clip.settings")}
          </button>
        </div>
      </div>

      {/* 工具栏：搜索 / 类型过滤 / 仅置顶 */}
      <div className="flex items-center gap-2 shrink-0 flex-wrap">
        <div className="flex items-center gap-1 flex-1 min-w-[200px] bg-white/5 border border-white/10 rounded-lg pl-2.5 pr-1 h-8">
          <Search className="w-3 h-3 text-slate-500 flex-shrink-0" />
          <input
            value={searchInput}
            onChange={(e) => onSearchChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSearch();
              if (e.key === "Escape") {
                setSearchInput("");
                setKeyword("");
              }
            }}
            placeholder={t("clip.searchPh")}
            className="flex-1 bg-transparent outline-none text-[11.5px] text-slate-200 placeholder:text-slate-500"
          />
          <button
            onClick={handleSearch}
            className="px-2 py-1 rounded-md bg-[var(--module-accent)] text-white text-[10.5px] font-semibold hover:opacity-90 cursor-pointer"              title={t("common.search")}
            >
            {t("common.search")}
          </button>
          {searchInput && (
            <button
              onClick={() => {
                setSearchInput("");
                setKeyword("");
              }}
              className="text-slate-500 hover:text-slate-300 cursor-pointer"
              title={t("clip.clearInput")}
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
        <div className="flex items-center gap-0.5 bg-white/5 border border-white/10 rounded-lg p-0.5">
          {[
            { k: "", t: t("clip.filterAll") },
            { k: "text", t: t("clip.filterText") },
            { k: "image", t: t("clip.filterImage") },
          ].map((f) => (
            <button
              key={f.k || "all"}
              onClick={() => setKind(f.k)}
              className={`px-2.5 py-1 rounded-md text-[10.5px] font-medium transition-all cursor-pointer ${
                kind === f.k ? "bg-[var(--module-accent)] text-white shadow-sm" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {f.t}
            </button>
          ))}
        </div>
        <button
          onClick={() => setPinnedOnly((v) => !v)}
          className={`px-2.5 py-1.5 rounded-lg border text-[10.5px] font-medium flex items-center gap-1.5 transition-all cursor-pointer ${
            pinnedOnly
              ? "bg-amber-500/15 border-amber-500/30 text-amber-300"
              : "bg-white/5 border-white/10 text-slate-400 hover:text-slate-200"
          }`}
        >
          <Pin className="w-3 h-3" /> {t("clip.pinnedOnly")}
        </button>
      </div>

      {/* 虚拟滚动列表 */}
      <div className="flex-grow min-h-0 overflow-y-auto" ref={listRef} onScroll={onListScroll}>
        {items.length === 0 && !loading && (
          <VexEmptyState
            title={t("clip.emptyTitle")}
            desc={keyword ? t("clip.emptyDescKw") : t("clip.emptyDesc")}
            tick={keyword ? t("clip.emptyTickKw") : t("clip.emptyTick")}
            avatarSize={40}
            className="!py-12"
          />
        )}
        <div className="relative" style={{ height: totalRows * ROW_H }}>
          {Array.from({ length: end - start }, (_, i) => {
            const idx = start + i;
            return (
              <div key={idx} className="absolute left-0 right-0 px-0.5 py-[3px]" style={{ top: idx * ROW_H, height: ROW_H }}>
                {idx < items.length ? (
                  <Row
                    item={items[idx]}
                    copied={copiedId === items[idx].id}
                    onPreview={() => openPreview(items[idx])}
                    onCopy={() => copyItem(items[idx].id)}
                    onPaste={() => pasteItem(items[idx].id)}
                    onPin={() => togglePin(items[idx])}
                    onDelete={() => deleteItem(items[idx].id)}
                  />
                ) : hasMore ? (
                  <div className="h-full flex items-center justify-center gap-2 text-[10.5px] text-slate-500">
                    {loading ? (
                      <>
                        <Loader2 className="w-3 h-3 animate-spin" /> {t("clip.loading")}
                      </>
                    ) : (
                      <button onClick={() => load(false)} className="hover:text-slate-300 transition cursor-pointer">
                        {t("clip.loadMore")}
                      </button>
                    )}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>

      {/* 预览弹窗 */}
      {preview && (
        <PreviewModal
          item={preview}
          onClose={closePreview}
          onCopy={() => copyItem(preview.id)}
          onPaste={() => pasteItem(preview.id)}
        />
      )}

      {/* 设置面板 */}
      {showSettings && settings && (
        <SharedModal
          open
          onClose={() => setShowSettings(false)}
          title={
            <span className="inline-flex items-center gap-2">
              <Settings2 className="w-4 h-4 text-[var(--module-accent)]" /> {t("clip.settingsTitle")}
            </span>
          }
          width={480}
          bodyClass="space-y-5"
          footer={
            <><SharedButton onClick={() => setShowSettings(false)}>{t("common.cancel")}</SharedButton>
<SharedButton onClick={saveSettings} variant="primary">{t("common.save")}</SharedButton>
            </>
          }
        >
            {/* 监控开关 */}
            <div className="flex items-center justify-between rounded-xl bg-white/[0.03] border border-white/10 p-3">
              <div>
                <p className="text-[12px] text-slate-200 font-medium">{t("clip.monitorOn")}</p>
                <p className="text-[10.5px] text-slate-500 mt-0.5">{t("clip.monitorHint")}</p>
              </div>
              <button
                onClick={() => setSettings({ ...settings, enabled: !settings.enabled })}
                className={`w-10 rounded-full relative transition-all cursor-pointer ${settings.enabled ? "bg-[var(--module-accent)]" : "bg-white/15"}`}
                style={{ height: 22 }}
                title={settings.enabled ? t("clip.off") : t("clip.on")}
              >
                <span
                  className={`absolute top-0.5 w-[18px] h-[18px] rounded-full bg-white shadow transition-all ${settings.enabled ? "left-[calc(100%-20px)]" : "left-0.5"}`}
                />
              </button>
            </div>

            {/* 数字设置 */}
            <div className="grid grid-cols-2 gap-3">
              <label className="block">
                <span className="text-[11px] text-slate-400">{t("clip.historyCap")}</span>
                <input
                  type="number"
                  min={50}
                  max={10000}
                  value={settings.maxItems}
                  onChange={(e) => setSettings({ ...settings, maxItems: Math.max(50, Math.min(10000, Number(e.target.value) || 1000)) })}
                  className="mt-1 w-full h-9 px-3 rounded-lg bg-white/5 border border-white/10 text-[12px] text-slate-200 outline-none focus:border-[var(--module-accent-ring)]"
                />
              </label>
              <div className="flex items-end pb-1">
                <span className="text-[10px] text-slate-500">{t("clip.historyHint")}</span>
              </div>
            </div>

            {/* 开关组 */}
            {([
              { k: "storeImages" as const, t: "保存图片", d: "复制图片时保存到历史" },
              { k: "ignoreBlank" as const, t: "忽略纯空白文本", d: "空白 / 空行不记录" },
              { k: "ignoreShort" as const, t: "忽略过短文本", d: "长度 ≤ 2 的文本不记录" },
            ]).map((opt) => (
              <div key={opt.k} className="flex items-center justify-between rounded-xl bg-white/[0.03] border border-white/10 p-3">
                <div>
                  <p className="text-[12px] text-slate-200 font-medium">{opt.t}</p>
                  <p className="text-[10.5px] text-slate-500 mt-0.5">{opt.d}</p>
                </div>
                <button
                  onClick={() => setSettings({ ...settings, [opt.k]: !settings[opt.k] })}
                  className={`w-10 rounded-full relative transition-all cursor-pointer ${settings[opt.k] ? "bg-[var(--module-accent)]" : "bg-white/15"}`}
                  style={{ height: 22 }}
                >
                  <span
                    className={`absolute top-0.5 w-[18px] h-[18px] rounded-full bg-white shadow transition-all ${settings[opt.k] ? "left-[calc(100%-20px)]" : "left-0.5"}`}
                  />
                </button>
              </div>
            ))}

            {/* 忽略规则（按来源程序） */}
            <div>
              <p className="text-[12px] text-slate-200 font-medium mb-2">忽略规则（按来源程序）</p>
              <p className="text-[10.5px] text-slate-500 mb-2">
                来自这些程序（如 password.exe、winscp.exe）的复制内容将不会被记录。
              </p>
              <div className="flex flex-wrap gap-1.5 mb-2">
                {ignoredApps.length === 0 && <span className="text-[11px] text-slate-600">暂无忽略规则</span>}
                {ignoredApps.map((a) => (
                  <span key={a} className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-white/5 border border-white/10 text-[10.5px] text-slate-300">
                    {a}
                    <button onClick={() => removeIgnoredApp(a)} className="text-slate-500 hover:text-red-400 cursor-pointer">
                      <X className="w-3 h-3" />
                    </button>
                  </span>
                ))}
              </div>
              <div className="flex gap-2">
                <input
                  value={newApp}
                  onChange={(e) => setNewApp(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && addIgnoredApp()}
                  placeholder="输入程序名，如 chrome.exe"
                  className="flex-1 h-9 px-3 rounded-lg bg-white/5 border border-white/10 text-[12px] text-slate-200 outline-none focus:border-[var(--module-accent-ring)]"
                />
                <button
                  onClick={addIgnoredApp}
                  className="px-3 h-9 rounded-lg bg-[var(--module-accent)] hover:opacity-85 text-[11px] font-semibold text-white transition-all cursor-pointer flex items-center gap-1"
                >
                  <Plus className="w-3.5 h-3.5" /> 添加
                </button>
              </div>
            </div>

        </SharedModal>
      )}
    </div>
  );
}
