import React, { useState, useEffect, useMemo, useRef } from "react";
import {
  Search,
  Plus,
  Folder,
  FolderPlus,
  Globe,
  Shield,
  Trash2,
  Edit2,
  ExternalLink,
  UploadCloud,
  FolderOpen,
  Copy,
  Check,
  FileText,
  X,
  ChevronRight,
  Bookmark,
  Layers,
  Sparkles,
  ScanSearch,
  LayoutGrid,
  Settings2,
  Loader2,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Classification,
  Item,
  ItemCheckResult,
  LauncherSetting,
} from "./types";
import { matchPinyin } from "./pinyin";
import CategoryModal from "./CategoryModal";
import AddItemModal from "./AddItemModal";

export default function LauncherPanel() {
  const [classifications, setClassifications] = useState<Classification[]>([]);
  const [activeParentId, setActiveParentId] = useState<number | null>(null);
  const [allItems, setAllItems] = useState<Item[]>([]);

  // Search state (Figure 2: Unified Search)
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchSelectedIndex, setSearchSelectedIndex] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Settings
  const [settings, setSettings] = useState<LauncherSetting>({
    showHideShortcutKey: "Alt+Space",
    itemIconSize: 32,
    itemColumnNumber: 0,
    cardDensity: "cozy",
    showItemName: true,
    iconBackgroundColor: false,
    itemFontSize: 12,
    itemRadius: 12,
    itemBorder: true,
    categoryFontSize: 12,
    categoryGap: 24,
  });

  // 检测状态：itemId -> 检测结果（红框标识）
  const [checking, setChecking] = useState(false);
  const [checkResults, setCheckResults] = useState<Record<number, { exists: boolean }>>({});
  const [missingCount, setMissingCount] = useState(0);
  // 检测进度：null 表示不在检测中
  const [checkProgress, setCheckProgress] = useState<{ done: number; total: number; name: string } | null>(null);

  // 视图设置面板开关
  const [viewSettingsOpen, setViewSettingsOpen] = useState(false);
  const viewSettingsRef = useRef<HTMLDivElement>(null);

  // Drag & Drop State
  const [isDragOver, setIsDragOver] = useState(false);
  const [targetDragSubId, setTargetDragSubId] = useState<number | null>(null);

  // Modals state
  const [categoryModalOpen, setCategoryModalOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<Classification | null>(null);
  const [categoryParentId, setCategoryParentId] = useState<number | null>(null);

  const [itemModalOpen, setItemModalOpen] = useState(false);
  const [editingItem, setEditingItem] = useState<Item | null>(null);
  const [targetClassificationId, setTargetClassificationId] = useState<number>(1);

  // Bookmark import modal
  const [bookmarkModalOpen, setBookmarkModalOpen] = useState(false);
  const [importingBookmark, setImportingBookmark] = useState(false);

  // Context Menus state
  const [itemContextMenu, setItemContextMenu] = useState<{
    x: number;
    y: number;
    item: Item;
  } | null>(null);

  const [categoryContextMenu, setCategoryContextMenu] = useState<{
    x: number;
    y: number;
    category: Classification;
  } | null>(null);

  // 子类目展开/收起：collapsedGroups 中存在的分类 id 表示已收起
  const [collapsedGroups, setCollapsedGroups] = useState<Set<number>>(new Set());
  const toggleGroupCollapse = (catId: number) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(catId)) next.delete(catId);
      else next.add(catId);
      return next;
    });
  };
  const isGroupCollapsed = (catId: number) => collapsedGroups.has(catId);

  // Toast / notification
  const [toastMsg, setToastMsg] = useState<string | null>(null);
  const showToast = (msg: string) => {
    setToastMsg(msg);
    setTimeout(() => setToastMsg(null), 2500);
  };

  // Load Data
  const loadData = async () => {
    try {
      const clsList = await invoke<Classification[]>("launcher_get_classifications");
      setClassifications(clsList);

      const items = await invoke<Item[]>("launcher_get_all_items");
      setAllItems(items);

      const loadedSettings = await invoke<LauncherSetting>("launcher_get_settings");
      setSettings(loadedSettings);

      // Default active parent category (first top-level classification)
      if (clsList.length > 0) {
        if (!activeParentId || !clsList.some((c) => c.id === activeParentId)) {
          setActiveParentId(clsList[0].id);
        }
      }
    } catch (e) {
      console.error("加载启动器数据失败:", e);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  // 从持久化的检测结果恢复红框标识（data.exists 为 false 的项目）
  useEffect(() => {
    if (allItems.length === 0) return;
    const restored: Record<number, { exists: boolean }> = {};
    let missing = 0;
    for (const it of allItems) {
      if (it.data.exists === false) {
        restored[it.id] = { exists: false };
        missing++;
      }
    }
    setCheckResults(restored);
    setMissingCount(missing);
  }, [allItems]);

  // Top-level categories (left sidebar)
  const topCategories = useMemo(() => {
    return classifications.filter((c) => !c.parentId);
  }, [classifications]);

  // Current active top-level category
  const activeTopCategory = useMemo(() => {
    return topCategories.find((c) => c.id === activeParentId) || topCategories[0];
  }, [topCategories, activeParentId]);

  // Sub-categories under current active parent category
  const subCategories = useMemo(() => {
    if (!activeTopCategory) return [];
    return classifications.filter((c) => c.parentId === activeTopCategory.id);
  }, [classifications, activeTopCategory]);

  // Items mapped by classification ID
  const itemsByClassification = useMemo(() => {
    const map = new Map<number, Item[]>();
    for (const item of allItems) {
      const list = map.get(item.classificationId) || [];
      list.push(item);
      map.set(item.classificationId, list);
    }
    return map;
  }, [allItems]);

  // Classification lookup map for name resolution
  const classificationMap = useMemo(() => {
    const map = new Map<number, Classification>();
    for (const c of classifications) {
      map.set(c.id, c);
    }
    return map;
  }, [classifications]);

  // Execute Item launch (单击秒开，打开后隐藏窗口)
  const handleExecuteItem = async (item: Item) => {
    try {
      await invoke("launcher_execute_item", {
        itemId: item.id > 0 ? item.id : null,
        item,
      });
      loadData();
      try {
        const win = getCurrentWindow();
        await win.hide();
      } catch (hideErr) {
        console.warn("Hide window on launch:", hideErr);
      }
    } catch (e) {
      showToast(`启动失败: ${e}`);
    }
  };

  // Save Category
  const handleSaveCategory = async (cat: Classification) => {
    await invoke("launcher_save_classification", { classification: cat });
    showToast("分类已保存");
    await loadData();
  };

  // Delete Category
  const handleDeleteCategory = async (id: number) => {
    if (!confirm("确定要删除此分类及其下的所有项目吗？")) return;
    await invoke("launcher_delete_classification", { id });
    showToast("分类已删除");
    await loadData();
  };

  // Save Item
  const handleSaveItem = async (it: Item) => {
    await invoke("launcher_save_item", { item: it });
    showToast("启动项已保存");
    await loadData();
  };

  // Delete Item
  const handleDeleteItem = async (id: number) => {
    await invoke("launcher_delete_item", { id });
    showToast("启动项已删除");
    await loadData();
  };

  // Import Browser Bookmarks
  const handleImportBookmarks = async (browser: "edge" | "chrome") => {
    setImportingBookmark(true);
    try {
      const res = await invoke<{ count: number; categoryId: number }>("launcher_import_browser_bookmarks", {
        browser,
        customPath: null,
      });
      setBookmarkModalOpen(false);
      await loadData();
      if (res && res.categoryId) {
        setActiveParentId(res.categoryId);
      }
      showToast(`成功导入 ${res?.count || 0} 个 ${browser === "edge" ? "Edge" : "Chrome"} 收藏夹书签`);
    } catch (e: any) {
      showToast(`导入失败: ${e}`);
    } finally {
      setImportingBookmark(false);
    }
  };

  // Process dropped paths with deduplication
  const isDroppingRef = useRef(false);
  const lastDropKeyRef = useRef("");
  const lastDropTimeRef = useRef(0);

  const targetDragSubIdRef = useRef<number | null>(null);
  const activeTopCategoryRef = useRef<Classification | undefined>(undefined);

  useEffect(() => {
    targetDragSubIdRef.current = targetDragSubId;
  }, [targetDragSubId]);

  useEffect(() => {
    activeTopCategoryRef.current = activeTopCategory;
  }, [activeTopCategory]);

  const processDroppedPaths = async (paths: string[], targetClassificationId: number) => {
    if (!paths || paths.length === 0) return;
    const now = Date.now();
    const dropKey = `${paths.slice().sort().join("|")}-${targetClassificationId}`;
    if (dropKey === lastDropKeyRef.current && now - lastDropTimeRef.current < 1200) {
      return;
    }
    lastDropKeyRef.current = dropKey;
    lastDropTimeRef.current = now;

    if (isDroppingRef.current) return;
    isDroppingRef.current = true;

    try {
      const newItems = await invoke<Item[]>("launcher_process_dropped_paths", {
        paths,
        classificationId: targetClassificationId,
      });
      showToast(`已成功添加 ${newItems.length} 个项目`);
      await loadData();
    } catch (err: any) {
      console.error("处理拖入路径失败:", err);
      showToast(`录入失败: ${err}`);
    } finally {
      isDroppingRef.current = false;
      setIsDragOver(false);
      setTargetDragSubId(null);
    }
  };

  // Native Tauri Window Drag & Drop listener (registered once on mount)
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setupTauriDragDrop = async () => {
      try {
        const win = getCurrentWindow();
        unlisten = await win.onDragDropEvent(async (event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            setIsDragOver(true);
          } else if (event.payload.type === "leave") {
            setIsDragOver(false);
            setTargetDragSubId(null);
          } else if (event.payload.type === "drop") {
            setIsDragOver(false);
            const paths = event.payload.paths;
            if (paths && paths.length > 0) {
              const targetId =
                targetDragSubIdRef.current ||
                (activeTopCategoryRef.current ? activeTopCategoryRef.current.id : 1);
              await processDroppedPaths(paths, targetId);
            }
            setTargetDragSubId(null);
          }
        });
      } catch (e) {
        console.error("Tauri drag-drop listener registration error:", e);
      }
    };

    setupTauriDragDrop();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // HTML5 Drag & Drop Fallback Handlers
  const handleHtml5DragOver = (e: React.DragEvent, subId?: number) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
    if (subId) setTargetDragSubId(subId);
  };

  const handleHtml5DragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  };

  const handleHtml5Drop = async (e: React.DragEvent, subId?: number) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);

    const targetId = subId || targetDragSubId || (activeTopCategory ? activeTopCategory.id : 1);
    const files = e.dataTransfer.files;
    if (files && files.length > 0) {
      const paths: string[] = [];
      for (let i = 0; i < files.length; i++) {
        // @ts-ignore
        const fullPath = files[i].path || files[i].name;
        if (fullPath) paths.push(fullPath);
      }
      if (paths.length > 0) {
        await processDroppedPaths(paths, targetId);
      }
    }
  };

  // Unified Search Filtering (Figure 2)
  // 支持：子串匹配（名称/目标/备注）+ 拼音首字母匹配（如 vsc → Visual Studio Code）
  const searchResults = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.trim().toLowerCase();
    return allItems.filter(
      (it) =>
        matchPinyin(it.name, q) ||
        (it.data.target || "").toLowerCase().includes(q) ||
        matchPinyin(it.data.remark || "", q)
    );
  }, [allItems, searchQuery]);

  // Open / Close Search
  const openSearch = () => {
    setIsSearchOpen(true);
    setSearchQuery("");
    setSearchSelectedIndex(0);
    setTimeout(() => {
      searchInputRef.current?.focus();
    }, 50);
  };

  const closeSearch = () => {
    setIsSearchOpen(false);
    setSearchQuery("");
  };

  // Keyboard shortcut listener (TEMPORARILY COMMENTED OUT FOR TESTING)
  /*
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // If any modal is active, do not handle launcher shortcuts
      if (categoryModalOpen || itemModalOpen || bookmarkModalOpen) {
        return;
      }

      const targetTag = (e.target as HTMLElement)?.tagName;
      const isTyping = targetTag === "INPUT" || targetTag === "TEXTAREA" || targetTag === "SELECT";

      if (e.key === "Escape") {
        if (isSearchOpen) {
          closeSearch();
          e.preventDefault();
        }
        return;
      }

      if (isTyping && !isSearchOpen) {
        return;
      }

      if ((e.ctrlKey && e.key.toLowerCase() === "f") || (e.key === "/" && !isTyping)) {
        e.preventDefault();
        openSearch();
        return;
      }

      if (isSearchOpen && searchResults.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSearchSelectedIndex((prev) => (prev + 1) % searchResults.length);
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          setSearchSelectedIndex((prev) => (prev - 1 + searchResults.length) % searchResults.length);
        } else if (e.key === "Enter") {
          e.preventDefault();
          const target = searchResults[searchSelectedIndex];
          if (target) {
            handleExecuteItem(target);
            closeSearch();
          }
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isSearchOpen, searchResults, searchSelectedIndex, categoryModalOpen, itemModalOpen, bookmarkModalOpen]);

  // Global hotkey event listener for Quick Search
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen("launcher-open-search", () => {
        openSearch();
      });
    };
    setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);
  */

  // Close context menus on global click
  useEffect(() => {
    const closeMenu = () => {
      setItemContextMenu(null);
      setCategoryContextMenu(null);
    };
    const onClickOutside = (e: MouseEvent) => {
      if (viewSettingsRef.current && !viewSettingsRef.current.contains(e.target as Node)) {
        setViewSettingsOpen(false);
      }
    };
    window.addEventListener("click", closeMenu);
    document.addEventListener("mousedown", onClickOutside);
    return () => {
      window.removeEventListener("click", closeMenu);
      document.removeEventListener("mousedown", onClickOutside);
    };
  }, []);

  // 视图设置：更新全局设置并保存
  const saveViewSettings = async (patch: Partial<LauncherSetting>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    try {
      await invoke("launcher_save_settings", { settings: next });
    } catch (e) {
      console.error("保存视图设置失败", e);
    }
  };

  // 监听检测进度事件（后端每完成一项 emit 一次，实时逐项呈现结果）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen<{
        done: number;
        total: number;
        itemId: number;
        name: string;
        exists: boolean;
        icon?: string | null;
        title?: string | null;
        stopped: boolean;
      }>("launcher-check-progress", (event) => {
        const p = event.payload;
        setCheckProgress({ done: p.done, total: p.total, name: p.name });

        // 用户停止：结束检测
        if (p.stopped) {
          setChecking(false);
          setCheckProgress(null);
          showToast("检测已停止");
          return;
        }

        // 立即呈现该项结果：缺失→红框；网页类→更新图标/标题
        setCheckResults((prev) => ({ ...prev, [p.itemId]: { exists: p.exists } }));
        setMissingCount((prev) => (p.exists ? prev : prev + 1));

        if (p.icon || p.title) {
          setAllItems((prev) =>
            prev.map((it) =>
              it.id === p.itemId
                ? {
                    ...it,
                    name: p.title ? p.title : it.name,
                    data: {
                      ...it.data,
                      icon: p.icon ? p.icon : it.data.icon,
                    },
                  }
                : it
            )
          );
        }
      });
    };
    setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // 停止检测
  const stopCheck = () => {
    invoke("launcher_stop_check").catch(() => {});
  };

  // 检测所有项目是否存在（网页类刷新图标，其余红框标识缺失）
  const runCheck = async () => {
    if (allItems.length === 0) return;
    setChecking(true);
    setCheckProgress({ done: 0, total: allItems.length, name: "" });
    setMissingCount(0);
    setCheckResults({});
    try {
      const results = await invoke<ItemCheckResult[]>("launcher_check_items", {
        items: allItems,
      });
      // 兜底：一次性同步所有结果（兼容未收到部分事件的情形）
      const newResults: Record<number, { exists: boolean }> = {};
      let missing = 0;
      for (const r of results) {
        newResults[r.itemId] = { exists: r.exists };
        if (!r.exists) missing++;
      }
      setCheckResults(newResults);
      setMissingCount(missing);
      showToast(`检测完成：${missing > 0 ? `${missing} 个项目不存在` : "全部项目正常"}`);
    } catch (e) {
      console.error("检测失败", e);
      showToast("检测失败，请重试");
    } finally {
      setChecking(false);
      setCheckProgress(null);
    }
  };

  // Name of current hover drag group for floating indicator
  const currentDragTargetName = useMemo(() => {
    if (targetDragSubId) {
      const sub = classificationMap.get(targetDragSubId);
      if (sub) return sub.name;
    }
    return activeTopCategory?.name || "当前分类";
  }, [targetDragSubId, classificationMap, activeTopCategory]);

  // ---- 全局视图设置：应用到所有分类的项目网格 ----
  const view = {
    iconSize: settings.itemIconSize ?? 32,
    columns: settings.itemColumnNumber ?? 0,
    density: settings.cardDensity ?? "cozy",
    showName: settings.showItemName ?? true,
    iconBg: settings.iconBackgroundColor ?? false,
    itemFontSize: settings.itemFontSize ?? 12,
    itemRadius: settings.itemRadius ?? 12,
    itemBorder: settings.itemBorder ?? true,
    categoryFontSize: settings.categoryFontSize ?? 12,
    categoryGap: settings.categoryGap ?? 24,
  };
  const gridStyle: React.CSSProperties = view.columns > 0
    ? { gridTemplateColumns: `repeat(${view.columns}, minmax(0, 1fr))` }
    : {};
  const gridClass = view.columns > 0
    ? "grid gap-2.5"
    : "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 xl:grid-cols-7 gap-2.5";
  const densityGap = view.density === "compact" ? "gap-1.5" : view.density === "spacious" ? "gap-3.5" : "gap-2.5";
  const densityCard = view.density === "compact"
    ? "px-2 py-1 gap-1.5"
    : view.density === "spacious"
      ? "px-3 py-2.5 gap-2.5"
      : "px-2.5 py-1.5 gap-2";
  // 卡片圆角/边框（受全局设置控制）
  const cardRadius = { borderRadius: view.itemRadius };
  const cardBorderClass = view.itemBorder
    ? ""
    : "!border-transparent";
  const itemNameStyle: React.CSSProperties = {
    fontSize: view.itemFontSize,
  };
  const categoryNameStyle: React.CSSProperties = {
    fontSize: view.categoryFontSize,
  };

  // 递归渲染分组：支持多级子分类（如浏览器收藏夹导入后的层级结构）
  const renderGroup = (cat: Classification): React.ReactNode => {
    const groupItems = itemsByClassification.get(cat.id) || [];
    const childGroups = classifications.filter((c) => c.parentId === cat.id);
    const isGroupHovered = targetDragSubId === cat.id;

    return (
      <div
        key={cat.id}
        className={`space-y-1.5 rounded-xl p-1.5 transition ${
          isGroupHovered ? "bg-[var(--module-accent-soft)] ring-1 ring-[var(--module-accent-ring)]" : ""
        }`}
        onDragOver={(e) => handleHtml5DragOver(e, cat.id)}
      >
        {/* Group Header (Left aligned with horizontal divider line) */}
        <div className="flex items-center justify-start relative py-0.5">
          <div className="absolute inset-0 flex items-center pointer-events-none">
            <div className="w-full border-t border-white/5" />
          </div>
          <div
            onClick={() => toggleGroupCollapse(cat.id)}
            onContextMenu={(e) => {
              e.preventDefault();
              setCategoryContextMenu({ x: e.clientX, y: e.clientY, category: cat });
            }}
            className="relative bg-[#0c101c] pr-4 pl-0.5 font-semibold text-slate-300 tracking-wide flex items-center gap-1 cursor-pointer hover:text-[var(--module-accent)] transition group-header"
            style={categoryNameStyle}
            title="点击展开/收起，右键管理此分组"
          >
            <ChevronRight
              className={`w-3 h-3 text-slate-500 transition-transform duration-150 flex-shrink-0 ${
                isGroupCollapsed(cat.id) ? "" : "rotate-90"
              }`}
            />
            {cat.data.icon ? (
              <span className="leading-none" style={{ fontSize: view.categoryFontSize + 2 }}>{cat.data.icon}</span>
            ) : (
              <span className="w-1.5 h-1.5 rounded-sm bg-gradient-to-br from-[var(--module-accent)] to-cyan-400 shadow-sm shadow-[var(--module-accent-ring)]" />
            )}
            <span>{cat.name}</span>
            {groupItems.length > 0 && (
              <span className="text-[10px] text-slate-500 font-normal ml-0.5">
                {groupItems.length}
              </span>
            )}
          </div>
        </div>

        {/* Group Items Grid (Horizontal: Icon + Name, Auto Column Grid) */}
        {groupItems.length > 0 && !isGroupCollapsed(cat.id) ? (
          <div className={`${gridClass} ${densityGap}`} style={gridStyle}>
            {groupItems.map((item) => (
              <div
                key={item.id}
                onClick={() => handleExecuteItem(item)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                }}
                className={`group relative rounded-xl border transition cursor-pointer min-w-0 flex items-center ${densityCard} ${cardBorderClass} ${
                  checkResults[item.id] && !checkResults[item.id].exists
                    ? "border-red-500/70 bg-red-500/10 hover:border-red-400 shadow-red-500/20 shadow-lg"
                    : "border-white/5 hover:border-[var(--module-accent-ring)] bg-white/[0.02] hover:bg-[var(--module-accent-soft)]"
                } active:scale-95`}
                style={cardRadius}
                title={
                  checkResults[item.id] && !checkResults[item.id].exists
                    ? `${item.name}（不存在）`
                    : item.name
                }
              >
                {item.data.runAsAdmin && (
                  <div className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-amber-400" />
                )}
                {checkResults[item.id] && !checkResults[item.id].exists && (
                  <div className="absolute -top-1 -left-1 w-2 h-2 rounded-full bg-red-500 shadow shadow-red-500/50" />
                )}

                {/* Icon (size from view settings) */}
                <div
                  className={`rounded-lg flex items-center justify-center flex-shrink-0 group-hover:scale-105 transition overflow-hidden ${
                    view.iconBg ? "bg-white/5" : "bg-transparent"
                  }`}
                  style={{ width: view.iconSize, height: view.iconSize }}
                >
                  {item.data.icon ? (
                    <img
                      src={item.data.icon}
                      className="object-contain"
                      style={{ width: view.iconSize * 0.82, height: view.iconSize * 0.82 }}
                      alt=""
                    />
                  ) : item.data.htmlIcon ? (
                    <span style={{ fontSize: view.iconSize * 0.5 }}>{item.data.htmlIcon}</span>
                  ) : item.itemType === 1 ? (
                    <Folder className="text-amber-400" style={{ width: view.iconSize * 0.55, height: view.iconSize * 0.55 }} />
                  ) : item.itemType === 2 ? (
                    <Globe className="text-blue-400" style={{ width: view.iconSize * 0.55, height: view.iconSize * 0.55 }} />
                  ) : (
                    <FileText className="text-[var(--module-accent)]" style={{ width: view.iconSize * 0.55, height: view.iconSize * 0.55 }} />
                  )}
                </div>

                {/* Name */}
                {view.showName && (
                  <span className="text-slate-200 truncate group-hover:text-[var(--module-accent)] transition min-w-0 flex-1 font-medium" style={itemNameStyle}>
                    {item.name}
                  </span>
                )}
              </div>
            ))}
          </div>
        ) : childGroups.length > 0 ? null : (
          <div
            onClick={() => {
              setEditingItem(null);
              setTargetClassificationId(cat.id);
              setItemModalOpen(true);
            }}
            className="border border-dashed border-white/5 hover:border-[var(--module-accent-ring)] rounded-xl py-3 text-center text-slate-600 hover:text-slate-400 text-[11px] cursor-pointer transition"
          >
            + 点击为此分组添加项目，或直接拖入 .exe / 文件夹 / 任意文件
          </div>
        )}

        {/* Child Groups (Nested, recursive) */}
        {childGroups.length > 0 && !isGroupCollapsed(cat.id) && (
          <div className="ml-2 pl-2.5 border-l border-white/5 space-y-2.5">
            {childGroups.map((child) => renderGroup(child))}
          </div>
        )}
      </div>
    );
  };

  return (
    <div
      className="w-full h-full flex flex-col bg-[#0b101b] text-slate-100 overflow-hidden relative font-sans"
      onDragOver={(e) => handleHtml5DragOver(e)}
      onDragLeave={handleHtml5DragLeave}
      onDrop={(e) => handleHtml5Drop(e)}
    >
      {/* Drag Over Overlay Toast / Indicator */}
      {isDragOver && (
        <div className="absolute inset-x-0 top-12 z-50 flex justify-center pointer-events-none animate-in fade-in slide-in-from-top-2 duration-150">
          <div className="bg-[color-mix(in_srgb,var(--module-accent)_90%,transparent)] backdrop-blur-md border border-[var(--module-accent)] text-white px-5 py-2.5 rounded-2xl shadow-2xl flex items-center gap-2.5">
            <UploadCloud className="w-5 h-5 text-[var(--module-accent)] animate-bounce" />
            <div className="text-xs">
              <span className="font-bold">松开鼠标立即添加至「{currentDragTargetName}」</span>
              <span className="text-[var(--module-accent)] text-[10px] ml-1.5">(支持 .exe、文件夹、任意文件、快捷方式)</span>
            </div>
          </div>
        </div>
      )}

      {/* Top Header Bar */}
      <div className="h-10 border-b border-white/5 px-4 flex items-center justify-between bg-black/20 flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm">{activeTopCategory?.data.icon || "📁"}</span>
          <span className="text-xs font-bold text-slate-200 tracking-wide">
            {activeTopCategory?.name || "快捷启动"}
          </span>
        </div>

        {/* Action Buttons */}
        <div className="flex items-center gap-1.5">
          {/* Search Trigger (Figure 2) */}
          <button
            onClick={() => {
              if (isSearchOpen) closeSearch();
              else openSearch();
            }}
            className={`px-2.5 py-1 rounded-lg text-xs flex items-center gap-1.5 border transition cursor-pointer ${
              isSearchOpen
                ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10 hover:text-white"
            }`}
            title="搜索全部快捷方式 (Ctrl+F)"
          >
            <Search className="w-3 h-3" />
            <span className="text-[11px]">搜索</span>
          </button>

          {/* Import Bookmarks */}
          <button
            onClick={() => setBookmarkModalOpen(true)}
            className="px-2.5 py-1 rounded-lg text-xs bg-white/5 border border-white/10 text-slate-300 hover:bg-white/10 hover:text-white transition cursor-pointer flex items-center gap-1.5"
            title="导入 Edge / Chrome 浏览器收藏夹"
          >
            <Bookmark className="w-3 h-3 text-amber-400" />
            <span className="text-[11px]">导入收藏夹</span>
          </button>

          {/* 检测 */}
          <button
            onClick={runCheck}
            disabled={checking || allItems.length === 0}
            className={`px-2.5 py-1 rounded-lg text-xs flex items-center gap-1.5 border transition cursor-pointer ${
              missingCount > 0
                ? "bg-red-600/20 border-red-500/40 text-red-300 hover:bg-red-600/30"
                : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10 hover:text-white"
            } ${checking ? "opacity-60 cursor-wait" : ""}`}
            title="检测项目是否存在：网页类自动更新链接图标，其余缺失项以红框标识"
          >
            {checking ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <ScanSearch className="w-3 h-3 text-emerald-400" />
            )}
            <span className="text-[11px]">{checking ? "检测中" : "检测"}</span>
            {missingCount > 0 && !checking && (
              <span className="w-3.5 h-3.5 rounded-full bg-red-500 text-[9px] text-white flex items-center justify-center font-bold">
                {missingCount}
              </span>
            )}
          </button>

          {/* 视图设置 */}
          <div className="relative" ref={viewSettingsRef}>
            <button
              onClick={() => setViewSettingsOpen((v) => !v)}
              className={`px-2.5 py-1 rounded-lg text-xs flex items-center gap-1.5 border transition cursor-pointer ${
                viewSettingsOpen
                  ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white"
                  : "bg-white/5 border-white/10 text-slate-300 hover:bg-white/10 hover:text-white"
              }`}
              title="调整项目尺寸与外观"
            >
              <LayoutGrid className="w-3 h-3 text-cyan-400" />
              <span className="text-[11px]">视图</span>
            </button>

            {viewSettingsOpen && (
              <div className="absolute right-0 mt-1 z-[150] w-64 bg-[#171d2e] border border-white/15 rounded-xl shadow-2xl shadow-black/50 p-3 space-y-3 max-h-[75vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
                <div className="flex items-center justify-between">
                  <span className="text-[11px] font-semibold text-slate-200 flex items-center gap-1">
                    <Settings2 className="w-3 h-3 text-[var(--module-accent)]" />
                    全局视图设置
                  </span>
                </div>

                {/* 图标大小 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">
                    图标大小：{settings.itemIconSize ?? 32}px
                  </label>
                  <input
                    type="range"
                    min={16}
                    max={64}
                    step={4}
                    value={settings.itemIconSize ?? 32}
                    onChange={(e) => saveViewSettings({ itemIconSize: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 网格列数 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">每行列数</label>
                  <div className="flex flex-wrap gap-1">
                    {[0, 3, 4, 5, 6, 7, 8].map((col) => (
                      <button
                        key={col}
                        onClick={() => saveViewSettings({ itemColumnNumber: col })}
                        className={`px-2 py-1 rounded-lg text-[10px] border transition cursor-pointer ${
                          (settings.itemColumnNumber ?? 0) === col
                            ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white"
                            : "bg-white/5 border-white/10 text-slate-400 hover:bg-white/10"
                        }`}
                      >
                        {col === 0 ? "自适应" : `${col}列`}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 卡片密度 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">卡片密度</label>
                  <div className="grid grid-cols-3 gap-1">
                    {[
                      { key: "compact", label: "紧凑" },
                      { key: "cozy", label: "舒适" },
                      { key: "spacious", label: "宽松" },
                    ].map((opt) => (
                      <button
                        key={opt.key}
                        onClick={() => saveViewSettings({ cardDensity: opt.key as LauncherSetting["cardDensity"] })}
                        className={`px-2 py-1.5 rounded-lg text-[10px] border transition cursor-pointer ${
                          (settings.cardDensity ?? "cozy") === opt.key
                            ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white"
                            : "bg-white/5 border-white/10 text-slate-400 hover:bg-white/10"
                        }`}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 项目文字大小 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">
                    项目文字大小：{settings.itemFontSize ?? 12}px
                  </label>
                  <input
                    type="range"
                    min={9}
                    max={20}
                    step={1}
                    value={settings.itemFontSize ?? 12}
                    onChange={(e) => saveViewSettings({ itemFontSize: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 分类文字大小 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">
                    分类文字大小：{settings.categoryFontSize ?? 12}px
                  </label>
                  <input
                    type="range"
                    min={9}
                    max={20}
                    step={1}
                    value={settings.categoryFontSize ?? 12}
                    onChange={(e) => saveViewSettings({ categoryFontSize: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 项目圆角 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">
                    项目圆角：{settings.itemRadius ?? 12}px
                  </label>
                  <input
                    type="range"
                    min={0}
                    max={20}
                    step={1}
                    value={settings.itemRadius ?? 12}
                    onChange={(e) => saveViewSettings({ itemRadius: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 分类间距 */}
                <div>
                  <label className="block text-[10px] text-slate-400 mb-1">
                    分类间距：{settings.categoryGap ?? 24}px
                  </label>
                  <input
                    type="range"
                    min={0}
                    max={32}
                    step={1}
                    value={settings.categoryGap ?? 24}
                    onChange={(e) => saveViewSettings({ categoryGap: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 开关 */}
                <div className="space-y-1.5">
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-[10px] text-slate-400">项目边框</span>
                    <button
                      onClick={() => saveViewSettings({ itemBorder: !(settings.itemBorder ?? true) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.itemBorder ?? true ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.itemBorder ?? true ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-[10px] text-slate-400">显示项目名称</span>
                    <button
                      onClick={() => saveViewSettings({ showItemName: !(settings.showItemName ?? true) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.showItemName ?? true ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.showItemName ?? true ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-[10px] text-slate-400">图标背景色块</span>
                    <button
                      onClick={() => saveViewSettings({ iconBackgroundColor: !(settings.iconBackgroundColor ?? false) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.iconBackgroundColor ?? false ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.iconBackgroundColor ?? false ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                </div>
              </div>
            )}
          </div>

          {/* Add Item */}
          <button
            onClick={() => {
              setEditingItem(null);
              setTargetClassificationId(activeTopCategory ? activeTopCategory.id : 1);
              setItemModalOpen(true);
            }}
            className="px-3 py-1 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white rounded-lg text-xs font-medium transition cursor-pointer shadow-md shadow-[var(--module-accent-ring)] flex items-center gap-1"
          >
            <Plus className="w-3 h-3" />
            <span className="text-[11px]">添加项目</span>
          </button>
        </div>
      </div>

      {/* 检测进度条（使用「启动」主题色：紫色） */}
      {checkProgress && (
        <div className="h-9 px-4 flex items-center gap-3 bg-[#0c101c] border-b border-white/5 flex-shrink-0 animate-in slide-in-from-top-2 duration-150">
          <div className="flex items-center gap-1.5 text-[11px] text-slate-400 whitespace-nowrap min-w-0">
            <Loader2 className="w-3.5 h-3.5 text-[var(--module-accent)] animate-spin flex-shrink-0" />
            <span className="truncate">
              {checkProgress.name
                ? <>正在检测 <span className="text-[var(--module-accent)]">{checkProgress.name}</span></>
                : "正在检测"}
            </span>
          </div>
          <div className="flex-1 h-1.5 bg-white/10 rounded-full overflow-hidden">
            <div
              className="h-full bg-gradient-to-r from-[var(--module-accent)] to-[var(--module-accent-strong)] rounded-full transition-all duration-200"
              style={{
                width: `${checkProgress.total > 0 ? Math.round((checkProgress.done / checkProgress.total) * 100) : 0}%`,
              }}
            />
          </div>
          <div className="text-[11px] text-slate-400 whitespace-nowrap tabular-nums">
            {checkProgress.done}/{checkProgress.total}
          </div>
          {checking && (
            <button
              onClick={stopCheck}
              className="px-2 py-0.5 rounded-md text-[10px] font-medium bg-red-600/20 border border-red-500/40 text-red-300 hover:bg-red-600/30 transition cursor-pointer whitespace-nowrap"
              title="停止检测"
            >
              停止
            </button>
          )}
        </div>
      )}

      {/* Main Split: Left Vertical Categories (128px) | Right Cards Flow */}
      <div className="flex-1 flex min-h-0 relative">
        {/* Left Vertical Categories (Fixed 128px) */}
        <div className="w-32 flex-shrink-0 border-r border-white/5 bg-[#090d16]/70 flex flex-col justify-between py-2 overflow-y-auto">
          <div className="space-y-1 px-1.5">
            {topCategories.map((cat) => {
              const isActive = cat.id === activeTopCategory?.id;

              return (
                <div
                  key={cat.id}
                  onClick={() => {
                    setActiveParentId(cat.id);
                    if (isSearchOpen) closeSearch();
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setCategoryContextMenu({ x: e.clientX, y: e.clientY, category: cat });
                  }}
                  className={`w-full px-3 py-2 rounded-xl text-xs flex items-center justify-between transition cursor-pointer group ${
                    isActive
                      ? "bg-slate-200 text-slate-900 font-bold shadow-md"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5 font-medium"
                  }`}
                >
                  <span className="truncate">{cat.name}</span>
                  <ChevronRight
                    className={`w-3.5 h-3.5 flex-shrink-0 transition ${
                      isActive ? "text-slate-900" : "opacity-40 group-hover:opacity-100"
                    }`}
                  />
                </div>
              );
            })}
          </div>

          {/* Add Category Button at bottom of sidebar */}
          <div className="px-1.5 pt-2 border-t border-white/5">
            <button
              onClick={() => {
                setEditingCategory(null);
                setCategoryParentId(null);
                setCategoryModalOpen(true);
              }}
              className="w-full py-1.5 px-2 rounded-xl text-slate-400 hover:text-slate-200 hover:bg-white/5 text-[11px] font-medium flex items-center justify-center gap-1 border border-dashed border-white/10 hover:border-white/20 transition cursor-pointer"
            >
              <Plus className="w-3 h-3" />
              <span>新建分类</span>
            </button>
          </div>
        </div>

        {/* Right Groups / Cards Stream */}
        <div
          className="flex-1 overflow-y-auto p-5 bg-[#0c101c]/90 flex flex-col"
          style={{ gap: view.categoryGap }}
        >
          {/* 1. If Category has Sub-Categories (recursively rendered) */}
          {subCategories.length > 0 ? (
            subCategories.map((sub) => renderGroup(sub))
          ) : null}

          {/* 2. Direct items under active category */}
          {(() => {
            const directItems = activeTopCategory
              ? itemsByClassification.get(activeTopCategory.id) || []
              : [];
            if (directItems.length === 0 && subCategories.length > 0) return null;

            return (
              <div className="space-y-2.5">
                {subCategories.length > 0 && (
                  <div className="flex items-center justify-start relative py-0.5">
                    <div className="absolute inset-0 flex items-center pointer-events-none">
                      <div className="w-full border-t border-white/5" />
                    </div>
                    <div className="relative bg-[#0c101c] pr-4 pl-0.5 font-semibold text-slate-400 tracking-wide flex items-center gap-1.5" style={categoryNameStyle}>
                      <span className="w-1.5 h-1.5 rounded-sm bg-gradient-to-br from-[var(--module-accent)] to-cyan-400 shadow-sm shadow-[var(--module-accent-ring)]" />
                      <span>其他项目</span>
                      {directItems.length > 0 && (
                        <span className="text-[10px] text-slate-600 font-normal">
                          {directItems.length}
                        </span>
                      )}
                    </div>
                  </div>
                )}

                {directItems.length > 0 ? (
                  <div className={`${gridClass} ${densityGap}`} style={gridStyle}>
                    {directItems.map((item) => (
                      <div
                        key={item.id}
                        onClick={() => handleExecuteItem(item)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                        }}
                        className={`group relative rounded-xl border transition cursor-pointer min-w-0 flex items-center ${densityCard} ${cardBorderClass} ${
                          checkResults[item.id] && !checkResults[item.id].exists
                            ? "border-red-500/70 bg-red-500/10 hover:border-red-400 shadow-lg shadow-red-500/20"
                            : "border-white/5 hover:border-[var(--module-accent-ring)] bg-white/[0.02] hover:bg-[var(--module-accent-soft)]"
                        } active:scale-95`}
                        style={cardRadius}
                        title={
                          checkResults[item.id] && !checkResults[item.id].exists
                            ? `${item.name}（不存在）`
                            : item.name
                        }
                      >
                        {item.data.runAsAdmin && (
                          <div className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-amber-400" />
                        )}
                        {checkResults[item.id] && !checkResults[item.id].exists && (
                          <div className="absolute -top-1 -left-1 w-2 h-2 rounded-full bg-red-500 shadow shadow-red-500/50" />
                        )}

                        <div
                          className={`rounded-lg flex items-center justify-center flex-shrink-0 group-hover:scale-105 transition overflow-hidden ${
                            view.iconBg ? "bg-white/5" : "bg-transparent"
                          }`}
                          style={{ width: view.iconSize, height: view.iconSize }}
                        >
                          {item.data.icon ? (
                            <img
                              src={item.data.icon}
                              className="object-contain"
                              style={{ width: view.iconSize * 0.82, height: view.iconSize * 0.82 }}
                              alt=""
                            />
                          ) : item.data.htmlIcon ? (
                            <span style={{ fontSize: view.iconSize * 0.5 }}>{item.data.htmlIcon}</span>
                          ) : item.itemType === 1 ? (
                            <Folder className="text-amber-400" style={{ width: view.iconSize * 0.55, height: view.iconSize * 0.55 }} />
                          ) : item.itemType === 2 ? (
                            <Globe className="text-blue-400" style={{ width: view.iconSize * 0.55, height: view.iconSize * 0.55 }} />
                          ) : (
                            <FileText className="text-[var(--module-accent)]" style={{ width: view.iconSize * 0.55, height: view.iconSize * 0.55 }} />
                          )}
                        </div>

                        {view.showName && (
                          <span className="text-slate-200 truncate group-hover:text-[var(--module-accent)] transition min-w-0 flex-1 font-medium" style={itemNameStyle}>
                            {item.name}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                ) : subCategories.length === 0 ? (
                  <div className="py-20 text-center text-slate-500 flex flex-col items-center justify-center">
                    <UploadCloud className="w-12 h-12 mb-3 opacity-30 text-[var(--module-accent)]" />
                    <p className="text-sm font-medium text-slate-300">当前分类暂无项目</p>
                    <p className="text-xs text-slate-500 mt-1 max-w-sm">
                      您可以直接将 .exe、文件夹、任意文件或快捷方式
                      <span className="text-[var(--module-accent)] font-medium">拖入此处</span>
                    </p>
                    <div className="flex items-center gap-2 mt-4">
                      <button
                        onClick={() => {
                          setEditingItem(null);
                          setTargetClassificationId(
                            activeTopCategory ? activeTopCategory.id : 1
                          );
                          setItemModalOpen(true);
                        }}
                        className="px-3.5 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-xs font-medium rounded-xl transition cursor-pointer flex items-center gap-1.5"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        添加项目
                      </button>
                      <button
                        onClick={() => {
                          setEditingCategory(null);
                          setCategoryParentId(
                            activeTopCategory ? activeTopCategory.id : null
                          );
                          setCategoryModalOpen(true);
                        }}
                        className="px-3.5 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 text-xs font-medium rounded-xl border border-white/10 transition cursor-pointer flex items-center gap-1.5"
                      >
                        <FolderPlus className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        新建子分组
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>
            );
          })()}
        </div>

        {/* Figure 2: Unified Search Overlay */}
        {isSearchOpen && (
          <div className="absolute inset-0 z-40 bg-[#0b101b]/95 backdrop-blur-md flex flex-col items-center p-6 animate-in fade-in zoom-in-95 duration-100">
            <div className="w-full max-w-[600px] h-full flex flex-col">
              {/* Search Input Box */}
              <div className="relative flex items-center mb-3">
                <Search className="w-4 h-4 absolute left-3.5 text-[var(--module-accent)] pointer-events-none" />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={searchQuery}
                  onChange={(e) => {
                    setSearchQuery(e.target.value);
                    setSearchSelectedIndex(0);
                  }}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "ArrowDown") {
                      e.preventDefault();
                      setSearchSelectedIndex((prev) => (prev + 1) % (searchResults.length || 1));
                    } else if (e.key === "ArrowUp") {
                      e.preventDefault();
                      setSearchSelectedIndex((prev) => (prev - 1 + (searchResults.length || 1)) % (searchResults.length || 1));
                    } else if (e.key === "Enter") {
                      e.preventDefault();
                      const target = searchResults[searchSelectedIndex];
                      if (target) {
                        handleExecuteItem(target);
                        closeSearch();
                      }
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      closeSearch();
                    }
                  }}
                  placeholder="搜索名称 / 拼音首字母 / 网址..."
                  className="w-full bg-white/5 border border-[var(--module-accent-ring)] rounded-xl pl-10 pr-10 py-2.5 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)] transition shadow-lg shadow-[var(--module-accent-ring)]"
                />
                <button
                  onClick={closeSearch}
                  className="absolute right-3 p-1 text-slate-400 hover:text-white rounded-lg hover:bg-white/10 transition cursor-pointer"
                  title="关闭搜索 (ESC)"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {/* Search Results List */}
              <div className="flex-1 overflow-y-auto space-y-1 pr-1">
              {searchResults.length > 0 ? (
                searchResults.map((item, idx) => {
                  const isSelected = idx === searchSelectedIndex;
                  const parentCategory = classificationMap.get(item.classificationId);
                  const parentName = parentCategory ? parentCategory.name : "";

                  return (
                    <div
                      key={item.id}
                      onClick={() => {
                        handleExecuteItem(item);
                        closeSearch();
                      }}
                      className={`flex items-center gap-3 px-3.5 py-2.5 rounded-xl text-xs transition cursor-pointer ${
                        isSelected
                          ? "bg-[var(--module-accent)] text-white font-medium shadow-md shadow-[var(--module-accent-ring)]"
                          : "text-slate-200 hover:bg-white/5"
                      }`}
                    >
                      {/* Icon */}
                      <div className="w-6 h-6 rounded-lg bg-white/10 flex items-center justify-center flex-shrink-0 overflow-hidden">
                        {item.data.icon ? (
                          <img src={item.data.icon} className="w-5 h-5 object-contain" alt="" />
                        ) : item.data.htmlIcon ? (
                          <span className="text-xs">{item.data.htmlIcon}</span>
                        ) : item.itemType === 1 ? (
                          <Folder className="w-4 h-4 text-amber-400" />
                        ) : item.itemType === 2 ? (
                          <Globe className="w-4 h-4 text-blue-400" />
                        ) : (
                          <FileText className="w-4 h-4 text-[var(--module-accent)]" />
                        )}
                      </div>

                      {/* Name + Parent Category Tag */}
                      <div className="flex items-center gap-2 min-w-0 flex-1">
                        <span className="truncate text-sm">{item.name}</span>
                        {parentName && (
                          <span
                            className={`text-xs px-1.5 py-0.5 rounded ${
                              isSelected ? "bg-black/20 text-[var(--module-accent)]" : "bg-white/5 text-slate-400"
                            }`}
                          >
                            ({parentName})
                          </span>
                        )}
                      </div>

                      {item.data.runAsAdmin && (
                        <Shield className={`w-3.5 h-3.5 ${isSelected ? "text-amber-300" : "text-amber-400"}`} />
                      )}
                    </div>
                  );
                })
              ) : searchQuery.trim() ? (
                <div className="py-16 text-center text-slate-500 text-xs">
                  未找到与「{searchQuery}」相关的快捷方式
                </div>
              ) : (
                <div className="py-12 text-center text-slate-500 text-xs space-y-1">
                  <p>输入拼音、程序名称或网址实时查找</p>
                  <p className="text-[11px] text-slate-600">支持 ↑ ↓ 方向键选择，Enter 键立即启动</p>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
      </div>

      {/* Item Context Menu */}
      {itemContextMenu && (
        <div
          style={{ top: itemContextMenu.y, left: itemContextMenu.x }}
          className="fixed z-[200] bg-[#171d2e] border border-white/15 rounded-xl shadow-2xl p-1.5 min-w-[160px] text-xs space-y-0.5 animate-in fade-in zoom-in-95 duration-100"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => {
              handleExecuteItem(itemContextMenu.item);
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-200 hover:bg-[var(--module-accent)] hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <ExternalLink className="w-3.5 h-3.5" />
            <span>打开</span>
          </button>
          <button
            onClick={() => {
              handleExecuteItem({
                ...itemContextMenu.item,
                data: { ...itemContextMenu.item.data, runAsAdmin: true },
              });
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-amber-300 hover:bg-amber-600 hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Shield className="w-3.5 h-3.5" />
            <span>以管理员身份运行</span>
          </button>
          {itemContextMenu.item.data.target && (
            <button
              onClick={() => {
                invoke("launcher_open_file_location", { path: itemContextMenu.item.data.target });
                setItemContextMenu(null);
              }}
              className="w-full px-3 py-1.5 rounded-lg text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
            >
              <FolderOpen className="w-3.5 h-3.5" />
              <span>打开文件所在位置</span>
            </button>
          )}
          {itemContextMenu.item.data.target && (
            <button
              onClick={() => {
                navigator.clipboard.writeText(itemContextMenu.item.data.target || "");
                showToast("已复制路径到剪贴板");
                setItemContextMenu(null);
              }}
              className="w-full px-3 py-1.5 rounded-lg text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
            >
              <Copy className="w-3.5 h-3.5" />
              <span>复制路径 / 网址</span>
            </button>
          )}
          <div className="h-px bg-white/10 my-1" />
          <button
            onClick={() => {
              setEditingItem(itemContextMenu.item);
              setTargetClassificationId(itemContextMenu.item.classificationId);
              setItemModalOpen(true);
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <Edit2 className="w-3.5 h-3.5" />
            <span>编辑属性</span>
          </button>
          <button
            onClick={() => {
              handleDeleteItem(itemContextMenu.item.id);
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-red-400 hover:bg-red-500 hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>删除项目</span>
          </button>
        </div>
      )}

      {/* Category Context Menu */}
      {categoryContextMenu && (
        <div
          style={{ top: categoryContextMenu.y, left: categoryContextMenu.x }}
          className="fixed z-[200] bg-[#171d2e] border border-white/15 rounded-xl shadow-2xl p-1.5 min-w-[150px] text-xs space-y-0.5 animate-in fade-in zoom-in-95 duration-100"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => {
              setEditingCategory(categoryContextMenu.category);
              setCategoryParentId(categoryContextMenu.category.parentId);
              setCategoryModalOpen(true);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-200 hover:bg-[var(--module-accent)] hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Edit2 className="w-3.5 h-3.5" />
            <span>编辑分类</span>
          </button>
          <button
            onClick={() => {
              setEditingCategory(null);
              setCategoryParentId(categoryContextMenu.category.id);
              setCategoryModalOpen(true);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <FolderPlus className="w-3.5 h-3.5" />
            <span>添加子分组</span>
          </button>
          <button
            onClick={() => {
              setEditingItem(null);
              setTargetClassificationId(categoryContextMenu.category.id);
              setItemModalOpen(true);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>添加项目到此分类</span>
          </button>
          <div className="h-px bg-white/10 my-1" />
          <button
            onClick={() => {
              handleDeleteCategory(categoryContextMenu.category.id);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-lg text-left text-red-400 hover:bg-red-500 hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>删除分类</span>
          </button>
        </div>
      )}

      {/* Browser Bookmarks Import Modal */}
      {bookmarkModalOpen && (
        <div
          className="fixed inset-0 z-[250] bg-black/60 backdrop-blur-sm flex items-center justify-center p-4 animate-in fade-in duration-100"
          onClick={() => setBookmarkModalOpen(false)}
        >
          <div
            className="w-full max-w-sm bg-[#171d2e] border border-white/15 rounded-2xl p-5 shadow-2xl space-y-4 text-xs"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between pb-2 border-b border-white/10">
              <div className="flex items-center gap-2">
                <Bookmark className="w-4 h-4 text-amber-400" />
                <h3 className="text-sm font-bold text-white">导入浏览器收藏夹</h3>
              </div>
              <button
                onClick={() => setBookmarkModalOpen(false)}
                className="text-slate-400 hover:text-white p-1"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <p className="text-[11px] text-slate-400 leading-relaxed">
              自动读取本机 Edge 或 Chrome 的书签文件，并自动同步其文件夹层级为启动器分类。
            </p>

            <div className="grid grid-cols-2 gap-3 pt-1">
              <button
                onClick={() => handleImportBookmarks("edge")}
                disabled={importingBookmark}
                className="p-3.5 rounded-xl border border-white/10 bg-white/5 hover:bg-[var(--module-accent-soft)] hover:border-[var(--module-accent-ring)] text-slate-200 transition cursor-pointer flex flex-col items-center text-center gap-2 disabled:opacity-50"
              >
                <Globe className="w-6 h-6 text-blue-400" />
                <span className="font-bold text-xs">Microsoft Edge</span>
                <span className="text-[9px] text-slate-500">一键导入书签</span>
              </button>

              <button
                onClick={() => handleImportBookmarks("chrome")}
                disabled={importingBookmark}
                className="p-3.5 rounded-xl border border-white/10 bg-white/5 hover:bg-[var(--module-accent-soft)] hover:border-[var(--module-accent-ring)] text-slate-200 transition cursor-pointer flex flex-col items-center text-center gap-2 disabled:opacity-50"
              >
                <Globe className="w-6 h-6 text-emerald-400" />
                <span className="font-bold text-xs">Google Chrome</span>
                <span className="text-[9px] text-slate-500">一键导入书签</span>
              </button>
            </div>

            {importingBookmark && (
              <p className="text-center text-[var(--module-accent)] text-xs animate-pulse">
                正在解析并导入书签，请稍候...
              </p>
            )}
          </div>
        </div>
      )}

      {/* Toast Notification */}
      {toastMsg && (
        <div className="fixed bottom-6 right-6 z-[300] bg-[var(--module-accent)] text-white text-xs px-4 py-2.5 rounded-xl shadow-xl flex items-center gap-2 animate-in fade-in slide-in-from-bottom-2 duration-150">
          <Check className="w-4 h-4" />
          <span>{toastMsg}</span>
        </div>
      )}

      {/* Category Modal */}
      {categoryModalOpen && (
        <CategoryModal
          key={editingCategory ? `cat-${editingCategory.id}` : `cat-new-${categoryParentId ?? 0}`}
          isOpen={categoryModalOpen}
          onClose={() => setCategoryModalOpen(false)}
          onSave={handleSaveCategory}
          editingCategory={editingCategory}
          parentCategories={classifications}
          currentParentId={categoryParentId}
        />
      )}

      {/* Add Item Modal */}
      {itemModalOpen && (
        <AddItemModal
          key={editingItem ? `item-${editingItem.id}` : `item-new-${targetClassificationId}`}
          isOpen={itemModalOpen}
          onClose={() => setItemModalOpen(false)}
          onSave={handleSaveItem}
          editingItem={editingItem}
          classificationId={targetClassificationId}
          classifications={classifications}
        />
      )}
    </div>
  );
}
