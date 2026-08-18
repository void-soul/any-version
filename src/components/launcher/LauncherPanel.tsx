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
  Command,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import {
  Classification,
  Item,
  LauncherSetting,
  ScannedProgram,
  ShortcutInfo,
} from "./types";
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
    openMode: "single_click",
    openAfterHide: false,
    itemLayout: "tile",
    columnCount: 0,
    density: "standard",
    iconSize: 48,
    nameDisplay: "show",
    defaultRunAsAdmin: false,
    webSearchSources: [],
  });

  // Drag & Drop
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

  // Toast / notification
  const [toastMsg, setToastMsg] = useState<string | null>(null);
  const showToast = (msg: string) => {
    setToastMsg(msg);
    setTimeout(() => setToastMsg(null), 2200);
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

  // Execute Item launch
  const handleExecuteItem = async (item: Item) => {
    try {
      await invoke("launcher_execute_item", {
        itemId: item.id > 0 ? item.id : null,
        item,
      });
      if (settings.openAfterHide) {
        // window will minimize/hide if setting enabled
      }
      loadData();
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
      const count = await invoke<number>("launcher_import_browser_bookmarks", {
        browser,
        customPath: null,
      });
      showToast(`成功导入 ${count} 个 ${browser === "edge" ? "Edge" : "Chrome"} 收藏夹书签`);
      setBookmarkModalOpen(false);
      await loadData();
    } catch (e: any) {
      showToast(`导入失败: ${e}`);
    } finally {
      setImportingBookmark(false);
    }
  };

  // Drag & Drop Handling
  const handleDragOver = (e: React.DragEvent, subId?: number) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
    if (subId) setTargetDragSubId(subId);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  };

  const handleDrop = async (e: React.DragEvent, subId?: number) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);

    const targetId = subId || targetDragSubId || (activeTopCategory ? activeTopCategory.id : null);
    if (!targetId) return;

    const files = e.dataTransfer.files;
    if (files && files.length > 0) {
      const itemsToAdd: Item[] = [];

      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        // @ts-ignore
        const fullPath = file.path || file.name;

        if (fullPath) {
          try {
            const info = await invoke<ShortcutInfo | null>("launcher_resolve_shortcut", {
              path: fullPath,
            });

            if (info) {
              itemsToAdd.push({
                id: 0,
                classificationId: targetId,
                name: info.name || file.name,
                itemType: info.isDir ? 1 : 0,
                data: {
                  target: info.targetPath,
                  params: info.arguments || undefined,
                  startLocation: info.workingDir || undefined,
                  runAsAdmin: false,
                  icon: info.iconBase64 || undefined,
                  openNumber: 0,
                  lastOpen: 0,
                },
                shortcutKey: null,
                globalShortcutKey: false,
                order: 0,
              });
            } else {
              const icon = await invoke<string | null>("launcher_extract_icon", { path: fullPath });
              itemsToAdd.push({
                id: 0,
                classificationId: targetId,
                name: file.name,
                itemType: 0,
                data: {
                  target: fullPath,
                  runAsAdmin: false,
                  icon: icon || undefined,
                  openNumber: 0,
                  lastOpen: 0,
                },
                shortcutKey: null,
                globalShortcutKey: false,
                order: 0,
              });
            }
          } catch (err) {
            console.error("解析拖拽文件失败:", err);
          }
        }
      }

      if (itemsToAdd.length > 0) {
        await invoke("launcher_batch_add_items", { items: itemsToAdd });
        showToast(`已录入 ${itemsToAdd.length} 个项目`);
        await loadData();
      }
    }
  };

  // Unified Search Filtering (Figure 2)
  const searchResults = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.trim().toLowerCase();
    return allItems.filter(
      (it) =>
        it.name.toLowerCase().includes(q) ||
        (it.data.target || "").toLowerCase().includes(q) ||
        (it.data.remark || "").toLowerCase().includes(q)
    );
  }, [allItems, searchQuery]);

  // Open Search helper
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

  // Keyboard shortcut listener for search and navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (isSearchOpen) {
          closeSearch();
          e.preventDefault();
        }
        return;
      }

      // Quick key to open search (Ctrl+F or '/' when not typing in input)
      if ((e.ctrlKey && e.key.toLowerCase() === "f") || (e.key === "/" && !(e.target instanceof HTMLInputElement))) {
        e.preventDefault();
        openSearch();
        return;
      }

      // In search mode, handle arrow keys and enter
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
  }, [isSearchOpen, searchResults, searchSelectedIndex]);

  // Close context menus on global click
  useEffect(() => {
    const closeMenu = () => {
      setItemContextMenu(null);
      setCategoryContextMenu(null);
    };
    window.addEventListener("click", closeMenu);
    return () => window.removeEventListener("click", closeMenu);
  }, []);

  return (
    <div
      className="w-full h-full flex flex-col bg-[#0b101b] text-slate-100 select-none overflow-hidden relative font-sans"
      onDragOver={(e) => handleDragOver(e)}
      onDragLeave={handleDragLeave}
      onDrop={(e) => handleDrop(e)}
    >
      {/* Top Header Bar */}
      <div className="h-10 border-b border-white/5 px-4 flex items-center justify-between bg-black/20 flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm">{activeTopCategory?.data.icon || "📁"}</span>
          <span className="text-xs font-bold text-slate-200 tracking-wide">
            {activeTopCategory?.name || "快捷启动"}
          </span>
          <span className="text-[10px] text-slate-500 ml-2">
            共 {allItems.length} 个项目 · 单击秒开
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
                ? "bg-purple-600 border-purple-400 text-white shadow-md shadow-purple-600/30"
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

          {/* Add Item */}
          <button
            onClick={() => {
              setEditingItem(null);
              setTargetClassificationId(activeTopCategory ? activeTopCategory.id : 1);
              setItemModalOpen(true);
            }}
            className="px-3 py-1 bg-purple-600 hover:bg-purple-500 text-white rounded-lg text-xs font-medium transition cursor-pointer shadow-md shadow-purple-600/20 flex items-center gap-1"
          >
            <Plus className="w-3 h-3" />
            <span className="text-[11px]">添加项目</span>
          </button>
        </div>
      </div>

      {/* Main Fig 1 Split: Left Vertical Categories | Right Cards Flow */}
      <div className="flex-1 flex min-h-0 relative">
        {/* Left Vertical Categories (Figure 1: 办公 > / 开发 > / 视频图片 > ...) */}
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
                  <ChevronRight className={`w-3.5 h-3.5 flex-shrink-0 transition ${isActive ? "text-slate-900" : "opacity-40 group-hover:opacity-100"}`} />
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

        {/* Right Groups / Cards Stream (Figure 1) */}
        <div className="flex-1 overflow-y-auto p-5 space-y-6 bg-[#0c101c]/90">
          {/* 1. If Category has Sub-Categories (Figure 1 layout) */}
          {subCategories.length > 0 ? (
            subCategories.map((sub) => {
              const groupItems = itemsByClassification.get(sub.id) || [];
              return (
                <div
                  key={sub.id}
                  className="space-y-2.5"
                  onDragOver={(e) => handleDragOver(e, sub.id)}
                  onDrop={(e) => handleDrop(e, sub.id)}
                >
                  {/* Group Header (Centered / Distinct like Fig 1: 沟通 / 工作 / 浏览器 / 远程 / 会议) */}
                  <div className="flex items-center justify-center relative py-1">
                    <div className="absolute inset-0 flex items-center pointer-events-none">
                      <div className="w-full border-t border-white/5" />
                    </div>
                    <div
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setCategoryContextMenu({ x: e.clientX, y: e.clientY, category: sub });
                      }}
                      className="relative bg-[#0c101c] px-4 text-xs font-bold text-slate-200 tracking-wider flex items-center gap-1.5 cursor-pointer hover:text-purple-300 transition"
                      title="右键管理此分组"
                    >
                      <span>{sub.name}</span>
                    </div>
                  </div>

                  {/* Group Items Grid (Left Icon + Right Name, High Density) */}
                  {groupItems.length > 0 ? (
                    <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 xl:grid-cols-7 gap-2.5">
                      {groupItems.map((item) => (
                        <div
                          key={item.id}
                          onClick={() => handleExecuteItem(item)}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                          }}
                          className="group relative flex items-center gap-2 px-2.5 py-1.5 rounded-xl border border-white/5 hover:border-purple-500/40 bg-white/[0.02] hover:bg-purple-600/10 active:scale-95 transition cursor-pointer min-w-0"
                          title={item.name}
                        >
                          {/* Admin indicator dot */}
                          {item.data.runAsAdmin && (
                            <div className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-amber-400" />
                          )}

                          {/* Left Icon (24px) */}
                          <div className="w-6 h-6 rounded-lg bg-white/5 flex items-center justify-center flex-shrink-0 group-hover:scale-105 transition overflow-hidden">
                            {item.data.icon ? (
                              <img src={item.data.icon} className="w-5 h-5 object-contain" alt="" />
                            ) : item.data.htmlIcon ? (
                              <span className="text-xs">{item.data.htmlIcon}</span>
                            ) : item.itemType === 1 ? (
                              <Folder className="w-4 h-4 text-amber-400" />
                            ) : item.itemType === 2 ? (
                              <Globe className="w-4 h-4 text-blue-400" />
                            ) : (
                              <FileText className="w-4 h-4 text-purple-400" />
                            )}
                          </div>

                          {/* Right Name ONLY (No description, no open count) */}
                          <span className="text-xs text-slate-200 truncate group-hover:text-purple-200 transition font-medium min-w-0 flex-1">
                            {item.name}
                          </span>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div
                      onClick={() => {
                        setEditingItem(null);
                        setTargetClassificationId(sub.id);
                        setItemModalOpen(true);
                      }}
                      className="border border-dashed border-white/5 hover:border-purple-500/30 rounded-xl py-3 text-center text-slate-600 hover:text-slate-400 text-[11px] cursor-pointer transition"
                    >
                      + 点击为此分组添加项目，或拖入文件
                    </div>
                  )}
                </div>
              );
            })
          ) : null}

          {/* 2. Direct items under active category (if no subcategories or unclassified) */}
          {(() => {
            const directItems = activeTopCategory
              ? itemsByClassification.get(activeTopCategory.id) || []
              : [];
            if (directItems.length === 0 && subCategories.length > 0) return null;

            return (
              <div className="space-y-2.5">
                {subCategories.length > 0 && (
                  <div className="flex items-center justify-center relative py-1">
                    <div className="absolute inset-0 flex items-center pointer-events-none">
                      <div className="w-full border-t border-white/5" />
                    </div>
                    <div className="relative bg-[#0c101c] px-4 text-xs font-bold text-slate-400 tracking-wider">
                      <span>其他项目</span>
                    </div>
                  </div>
                )}

                {directItems.length > 0 ? (
                  <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 xl:grid-cols-7 gap-2.5">
                    {directItems.map((item) => (
                      <div
                        key={item.id}
                        onClick={() => handleExecuteItem(item)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                        }}
                        className="group relative flex items-center gap-2 px-2.5 py-1.5 rounded-xl border border-white/5 hover:border-purple-500/40 bg-white/[0.02] hover:bg-purple-600/10 active:scale-95 transition cursor-pointer min-w-0"
                        title={item.name}
                      >
                        {item.data.runAsAdmin && (
                          <div className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-amber-400" />
                        )}

                        <div className="w-6 h-6 rounded-lg bg-white/5 flex items-center justify-center flex-shrink-0 group-hover:scale-105 transition overflow-hidden">
                          {item.data.icon ? (
                            <img src={item.data.icon} className="w-5 h-5 object-contain" alt="" />
                          ) : item.data.htmlIcon ? (
                            <span className="text-xs">{item.data.htmlIcon}</span>
                          ) : item.itemType === 1 ? (
                            <Folder className="w-4 h-4 text-amber-400" />
                          ) : item.itemType === 2 ? (
                            <Globe className="w-4 h-4 text-blue-400" />
                          ) : (
                            <FileText className="w-4 h-4 text-purple-400" />
                          )}
                        </div>

                        <span className="text-xs text-slate-200 truncate group-hover:text-purple-200 transition font-medium min-w-0 flex-1">
                          {item.name}
                        </span>
                      </div>
                    ))}
                  </div>
                ) : subCategories.length === 0 ? (
                  <div className="py-20 text-center text-slate-500 flex flex-col items-center justify-center">
                    <UploadCloud className="w-12 h-12 mb-3 opacity-30 text-purple-400" />
                    <p className="text-sm font-medium text-slate-300">当前分类暂无项目</p>
                    <p className="text-xs text-slate-500 mt-1 max-w-sm">
                      您可以直接将文件从桌面拖入此处，或右键创建子分组
                    </p>
                    <div className="flex items-center gap-2 mt-4">
                      <button
                        onClick={() => {
                          setEditingItem(null);
                          setTargetClassificationId(activeTopCategory ? activeTopCategory.id : 1);
                          setItemModalOpen(true);
                        }}
                        className="px-3.5 py-1.5 bg-purple-600 hover:bg-purple-500 text-white text-xs font-medium rounded-xl transition cursor-pointer flex items-center gap-1.5"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        添加项目
                      </button>
                      <button
                        onClick={() => {
                          setEditingCategory(null);
                          setCategoryParentId(activeTopCategory ? activeTopCategory.id : null);
                          setCategoryModalOpen(true);
                        }}
                        className="px-3.5 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 text-xs font-medium rounded-xl border border-white/10 transition cursor-pointer flex items-center gap-1.5"
                      >
                        <FolderPlus className="w-3.5 h-3.5 text-purple-400" />
                        新建子分组
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>
            );
          })()}
        </div>

        {/* Figure 2: Unified Search Overlay (Exact Fig 2 Replica) */}
        {isSearchOpen && (
          <div className="absolute inset-0 z-40 bg-[#0b101b]/95 backdrop-blur-md flex flex-col p-4 animate-in fade-in zoom-in-95 duration-100">
            {/* Search Input Box */}
            <div className="relative flex items-center mb-3">
              <Search className="w-4 h-4 absolute left-3.5 text-purple-400 pointer-events-none" />
              <input
                ref={searchInputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => {
                  setSearchQuery(e.target.value);
                  setSearchSelectedIndex(0);
                }}
                placeholder="键入关键词搜索所有快捷方式..."
                className="w-full bg-white/5 border border-purple-500/50 rounded-xl pl-10 pr-10 py-2.5 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-purple-400 transition shadow-lg shadow-purple-600/10"
              />
              <button
                onClick={closeSearch}
                className="absolute right-3 p-1 text-slate-400 hover:text-white rounded-lg hover:bg-white/10 transition cursor-pointer"
                title="关闭搜索 (ESC)"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* Search Results List (Figure 2: [Icon] [Name (Classification)]) */}
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
                          ? "bg-purple-600 text-white font-medium shadow-md shadow-purple-600/30"
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
                          <FileText className="w-4 h-4 text-purple-400" />
                        )}
                      </div>

                      {/* Name + Parent Category Tag (Fig 2) */}
                      <div className="flex items-center gap-2 min-w-0 flex-1">
                        <span className="truncate text-sm">{item.name}</span>
                        {parentName && (
                          <span
                            className={`text-xs px-1.5 py-0.5 rounded ${
                              isSelected ? "bg-black/20 text-purple-200" : "bg-white/5 text-slate-400"
                            }`}
                          >
                            ({parentName})
                          </span>
                        )}
                      </div>

                      {/* Admin icon badge if enabled */}
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
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-200 hover:bg-purple-600 hover:text-white flex items-center gap-2 cursor-pointer"
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
            className="w-full px-3 py-1.5 rounded-lg text-left text-slate-200 hover:bg-purple-600 hover:text-white flex items-center gap-2 cursor-pointer"
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
                className="p-3.5 rounded-xl border border-white/10 bg-white/5 hover:bg-purple-600/20 hover:border-purple-500 text-slate-200 transition cursor-pointer flex flex-col items-center text-center gap-2 disabled:opacity-50"
              >
                <Globe className="w-6 h-6 text-blue-400" />
                <span className="font-bold text-xs">Microsoft Edge</span>
                <span className="text-[9px] text-slate-500">一键导入书签</span>
              </button>

              <button
                onClick={() => handleImportBookmarks("chrome")}
                disabled={importingBookmark}
                className="p-3.5 rounded-xl border border-white/10 bg-white/5 hover:bg-purple-600/20 hover:border-purple-500 text-slate-200 transition cursor-pointer flex flex-col items-center text-center gap-2 disabled:opacity-50"
              >
                <Globe className="w-6 h-6 text-emerald-400" />
                <span className="font-bold text-xs">Google Chrome</span>
                <span className="text-[9px] text-slate-500">一键导入书签</span>
              </button>
            </div>

            {importingBookmark && (
              <p className="text-center text-purple-400 text-xs animate-pulse">
                正在解析并导入书签，请稍候...
              </p>
            )}
          </div>
        </div>
      )}

      {/* Toast Notification */}
      {toastMsg && (
        <div className="fixed bottom-6 right-6 z-[300] bg-purple-600 text-white text-xs px-4 py-2.5 rounded-xl shadow-xl flex items-center gap-2 animate-in fade-in slide-in-from-bottom-2 duration-150">
          <Check className="w-4 h-4" />
          <span>{toastMsg}</span>
        </div>
      )}

      {/* Category Modal */}
      <CategoryModal
        isOpen={categoryModalOpen}
        onClose={() => setCategoryModalOpen(false)}
        onSave={handleSaveCategory}
        editingCategory={editingCategory}
        parentCategories={classifications}
        currentParentId={categoryParentId}
      />

      {/* Add Item Modal */}
      <AddItemModal
        isOpen={itemModalOpen}
        onClose={() => setItemModalOpen(false)}
        onSave={handleSaveItem}
        editingItem={editingItem}
        classificationId={targetClassificationId}
        classifications={classifications}
      />
    </div>
  );
}
