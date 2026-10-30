import { useLauncherMenuDismissal } from "./useLauncherMenuDismissal";
import { LauncherCheckProgress } from "./LauncherCheckProgress";
import { LauncherCollapseButton } from "./LauncherCollapseButton";
import { LauncherCleanupButton } from "./LauncherCleanupButton";
import { LauncherDropIndicator } from "./LauncherDropIndicator";
import { LauncherFolderItems } from "./LauncherFolderItems";
import { LauncherCheckButton } from "./LauncherCheckButton";
import { useLauncherNavigation } from "./useLauncherNavigation";
import { buildLauncherAppearance } from "./buildLauncherAppearance";
import { useLauncherBackup } from "./useLauncherBackup";
import { useLauncherCatalog } from "./useLauncherCatalog";
import { useLauncherExecution } from "./useLauncherExecution";
import { useLauncherFolder } from "./useLauncherFolder";
import { useLauncherItemChecks } from "./useLauncherItemChecks";
import { useLauncherSearch } from "./useLauncherSearch";
import { useLauncherDragDrop } from "./useLauncherDragDrop";
import { useLauncherCategoryActions } from "./useLauncherCategoryActions";
import { LauncherDeleteCategoryDialog } from "./LauncherDeleteCategoryDialog";
import { LauncherMoveCategoriesDialog } from "./LauncherMoveCategoriesDialog";
import { LauncherCategoryMenu } from "./LauncherCategoryMenu";
import { LauncherFolderMenu } from "./LauncherFolderMenu";
import { LauncherItemMenu } from "./LauncherItemMenu";
import { LauncherSearchOverlay } from "./LauncherSearchOverlay";
import { LauncherUsageStats } from "./LauncherUsageStats";
import { LauncherAggregateItems } from "./LauncherAggregateItems";
import { LauncherCategorySidebar } from "./LauncherCategorySidebar";
import { LauncherViewSettings } from "./LauncherViewSettings";
import { useState, useMemo, useRef } from "react";
import { patchAppSettings } from "../../services/appSettings";
import { Search, Plus, Check, ChevronRight, BarChart3, RefreshCw, Download, Upload, Database } from "lucide-react";
import { DndContext, DragOverlay } from "@dnd-kit/core";
import { SortableContext, horizontalListSortingStrategy } from "@dnd-kit/sortable";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Classification, Item, LauncherSetting } from "./types";
import { sortLauncherItemsByUsage } from "./usageStats";
import CategoryModal from "./CategoryModal";
import AddItemModal from "./AddItemModal";
import VexGlowAvatar from "../VexGlowAvatar";
import VexEmptyState from "../VexEmptyState";
import VexGreeting from "../VexGreeting";
import { cardClass, customCollisionDetection } from "./LauncherPanel.model";
import { Droppable } from "./LauncherPanel.Droppable";
import { SortableItem } from "./LauncherPanel.SortableItem";
import { ItemCardBody } from "./LauncherPanel.ItemCardBody";

export default function LauncherPanel() {
  const { t } = useTranslation();

  // Search state (Figure 2: Unified Search)
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchSelectedIndex, setSearchSelectedIndex] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // 视图设置面板开关
  const [viewSettingsOpen, setViewSettingsOpen] = useState(false);
  const [usageStatsOpen, setUsageStatsOpen] = useState(false);

  const viewSettingsRef = useRef<HTMLDivElement>(null);

  // Modals state
  const [categoryModalOpen, setCategoryModalOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<Classification | null>(null);
  const [categoryParentId, setCategoryParentId] = useState<number | null>(null);

  const [itemModalOpen, setItemModalOpen] = useState(false);
  const [editingItem, setEditingItem] = useState<Item | null>(null);
  const [targetClassificationId, setTargetClassificationId] = useState<number>(1);

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
  // 标记是否已按分类的「默认收缩全部子分组」初始化过一次折叠状态（避免重复刷新后反复重置用户手动展开）
  const collapseInitRef = useRef(false);
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
  const {
    classifications,
    activeParentId,
    setActiveParentId,
    allItems,
    setAllItems,
    allItemsRef,
    settings,
    setSettings,
    topCategories,
    loadData,
  } = useLauncherCatalog({
    collapseInitRef,
    setCollapsedGroups,
  });

  // Current active top-level category
  const {
    activeTopCategory,
    subCategories,
    isFolderCategory,
    isAggregateCategory,
    activeFolderPath,
    webSources,
    activeSubIds,
    isAllCollapsed,
    handleToggleCollapseAll,
  } = useLauncherNavigation({
    topCategories,
    activeParentId,
    classifications,
    settings,
    collapsedGroups,
    setCollapsedGroups,
  });

  // Items mapped by classification ID
  // 开启「只显示有效项目」时过滤掉检测为不存在（data.exists === false）的项目；
  // 未检测过（exists 为 null）视为有效，避免误藏。
  const visibleItems = useMemo(() => {
    if (!settings.showOnlyValid) return allItems;
    return allItems.filter((it) => it.data.exists !== false);
  }, [allItems, settings.showOnlyValid]);

  const itemsByClassification = useMemo(() => {
    const map = new Map<number, Item[]>();
    for (const item of visibleItems) {
      const list = map.get(item.classificationId) || [];
      list.push(item);
      map.set(item.classificationId, list);
    }
    return map;
  }, [visibleItems]);

  // 聚合分类：跨全部分类挑选项目，按启动次数 / 最近启动 / 名称排序后取前 N 个
  const aggregateItems = useMemo(() => {
    if (!isAggregateCategory) return [] as Item[];
    const limit = Math.max(1, activeTopCategory?.data?.aggregateItemCount ?? 20);
    const sort = activeTopCategory?.data?.aggregateSort || "openNumber";
    // 「最常打开 / 最近启动」只聚合真正启动过的项目；按名称排序时展示全部
    const list = visibleItems.filter(
      (it) =>
        sort === "name" ||
        (it.data.openNumber ?? 0) > 0 ||
        (it.data.lastOpen ?? 0) > 0
    );
    if (sort === "name") {
      list.sort((a, b) => a.name.localeCompare(b.name, "zh-Hans-CN"));
    } else if (sort === "lastOpen") {
      list.sort((a, b) => (b.data.lastOpen ?? 0) - (a.data.lastOpen ?? 0));
    } else {
      list.sort(
        (a, b) =>
          (b.data.openNumber ?? 0) - (a.data.openNumber ?? 0) ||
          (b.data.lastOpen ?? 0) - (a.data.lastOpen ?? 0)
      );
    }
    return list.slice(0, limit);
  }, [isAggregateCategory, visibleItems, activeTopCategory]);

  // 关联文件夹分类：扫描结果转成只读卡片（id 为负数，仅用于渲染 key）
  const {
    folderItems,
    folderLoading,
    folderError,
    setFolderReloadKey,
    folderContextMenu,
    setFolderContextMenu,
    folderVirtualItems,
    addFolderItemToClassification,
  } = useLauncherFolder({
    isFolderCategory,
    activeTopCategory,
    activeFolderPath,
    showToast,
    loadData,
  });

  // Classification lookup map for name resolution
  const classificationMap = useMemo(() => {
    const map = new Map<number, Classification>();
    for (const c of classifications) {
      map.set(c.id, c);
    }
    return map;
  }, [classifications]);

  // Execute Item launch (单击秒开，打开后隐藏窗口)
  const {
    handleExecuteItem,
    launchVirtual,
  } = useLauncherExecution({
    loadData,
    showToast,
    classificationMap,
    setActiveParentId,
    classifications,
    setIsSearchOpen,
    setSearchQuery,
    isSearchOpen,
    allItems,
    activeTopCategory,
  });

  // 导出 / 导入整套启动器数据
  const {
    backupBusy,
    handleExportBackup,
    handleImportBackup,
  } = useLauncherBackup({
    showToast,
    loadData,
  });

  // Save Category
  const {
    moveItemsModalOpen,
    setMoveItemsModalOpen,
    moveItemsSource,
    moveItemsTarget,
    setMoveItemsTarget,
    moveItemsLoading,
    handleSaveCategory,
    pendingDeleteCategory,
    setPendingDeleteCategory,
    deletingCategory,
    deleteCascade,
    setDeleteCascade,
    deleteTarget,
    setDeleteTarget,
    deleteNewName,
    setDeleteNewName,
    scopeOf,
    handleDeleteCategory,
    confirmDeleteCategory,
    handleSaveItem,
    handleDeleteItem,
    moveItemsSourceCount,
    moveItemsTargetList,
    openMoveItemsModal,
    handleMoveItemsConfirm,
  } = useLauncherCategoryActions({
    showToast,
    loadData,
    classifications,
    setCategoryContextMenu,
  });

  // Process dropped paths with deduplication
  const {
    isDragOver,
    setIsDragOver,
    targetDragSubId,
    setTargetDragSubId,
    activeDragItem,
    sensors,
    justDraggedAtRef,
    processDroppedPaths,
    handleHtml5DragOver,
    handleHtml5DragLeave,
    handleHtml5Drop,
    handleDragStart,
    handleDragOver,
    handleDragEnd,
    handleDragCancel,
  } = useLauncherDragDrop({
    topCategories,
    activeTopCategory,
    setAllItems,
    showToast,
    setActiveParentId,
    allItems,
    allItemsRef,
    classificationMap,
    loadData,
  });

  /** 分类（含祖先）是否在「全局快速搜索」中被排除。 */
  const {
    searchQuickActions,
    searchResults,
    openFileHit,
    effectiveFileHits,
    searchTotal,
    openSearch,
    closeSearch,
  } = useLauncherSearch({
    classificationMap,
    searchQuery,
    webSources,
    visibleItems,
    isSearchOpen,
    setIsSearchOpen,
    setSearchQuery,
    setSearchSelectedIndex,
    searchInputRef,
  });

  const usageStats = useMemo(() => sortLauncherItemsByUsage(allItems), [allItems]);

  // Keyboard shortcut listener (TEMPORARILY COMMENTED OUT FOR TESTING)

  // Close context menus on global click
  useLauncherMenuDismissal({
    setItemContextMenu,
    setCategoryContextMenu,
    setFolderContextMenu,
    viewSettingsRef,
    setViewSettingsOpen,
  });

  // 视图设置：更新全局设置并保存
  const saveViewSettings = async (patch: Partial<LauncherSetting>) => {
    try {
      setSettings(await patchAppSettings(patch));
    } catch (e) {
      console.error("保存视图设置失败", e);
    }
  };

  // 停止检测
  const {
    checking,

    checkResults,

    missingCount,

    checkProgress,

    stopCheck,
    cleanupInvalidItems,
    runCheck,
  } = useLauncherItemChecks({
    allItems,
    showToast,
    setAllItems,
  });

  // Name of current hover drag group for floating indicator
  const currentDragTargetName = useMemo(() => {
    if (targetDragSubId) {
      const sub = classificationMap.get(targetDragSubId);
      if (sub) return sub.name;
    }
    return activeTopCategory?.name || t("launcher.currentCat");
  }, [targetDragSubId, classificationMap, activeTopCategory]);

  // ---- 全局视图设置：应用到所有分类的项目网格 ----
  const appearance = buildLauncherAppearance({
    settings,
  });
  const {
    view,
    gridStyle,
    gridClass,
    densityGap,
    categoryNameStyle,
    itemView,
  } = appearance;

  // 递归渲染分组：支持多级子分类（如浏览器收藏夹导入后的层级结构）
  const renderGroup = (cat: Classification): React.ReactNode => {
    const groupItems = itemsByClassification.get(cat.id) || [];
    const childGroups = classifications.filter((c) => c.parentId === cat.id);
    const isGroupHovered = targetDragSubId === cat.id;

    return (
      <Droppable
        key={cat.id}
        id={`cat:${cat.id}`}
        dataCatId={cat.id}
        className={`space-y-1.5 rounded-card p-1.5 transition ${
          isGroupHovered ? "bg-[var(--module-accent-soft)] ring-1 ring-[var(--module-accent-ring)]" : ""
        }`}
        onDragOver={(e) => handleHtml5DragOver(e, cat.id)}
        onDrop={(e) => handleHtml5Drop(e, cat.id)}
      >
        {/* Group Header (Left aligned with horizontal divider line) */}
        <div className="flex items-center justify-start relative py-0.5">
          <div className="absolute inset-0 flex items-center pointer-events-none">
            <div className="w-full border-t border-white/5" />
          </div>
          <div
            onClick={() => {
              if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发
              toggleGroupCollapse(cat.id);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              setCategoryContextMenu({ x: e.clientX, y: e.clientY, category: cat });
            }}
            className="relative bg-white/[0.04] backdrop-blur-sm pr-4 pl-0.5 font-semibold text-slate-300 tracking-wide flex items-center gap-1 cursor-pointer hover:text-[var(--module-accent)] transition group-header"
            style={categoryNameStyle}
            title={t("launcher.clickManageGroup")}
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
              <span className="text-tiny text-slate-500 font-normal ml-0.5">
                {groupItems.length}
              </span>
            )}
          </div>
        </div>

        {/* Group Items Grid (Horizontal: Icon + Name, Auto Column Grid) */}
        {!isGroupCollapsed(cat.id) && groupItems.length > 0 ? (
          <SortableContext
            items={groupItems.map((i) => `item:${i.id}`)}
            strategy={horizontalListSortingStrategy}
          >
            <div className={`${gridClass} ${densityGap}`} style={gridStyle}>
              {groupItems.map((item) => (
                <SortableItem
                  key={item.id}
                  item={item}
                  view={itemView}
                  checkResults={checkResults}
                  onClick={() => {
                    if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发启动
                    handleExecuteItem(item);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                  }}
                />
              ))}
            </div>
          </SortableContext>
        ) : !isGroupCollapsed(cat.id) && groupItems.length === 0 && childGroups.length === 0 ? (
          <div
            onClick={() => {
              if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发
              setEditingItem(null);
              setTargetClassificationId(cat.id);
              setItemModalOpen(true);
            }}
            className="border border-dashed border-white/5 hover:border-[var(--module-accent-ring)] rounded-card min-h-[64px] flex items-center justify-center py-3 text-center text-slate-600 hover:text-slate-400 text-caption cursor-pointer transition"
          >
            + {t("launcher.clickAddToGroup")}
          </div>
        ) : null}

        {/* Child Groups (Nested, recursive) */}
        {childGroups.length > 0 && !isGroupCollapsed(cat.id) && (
          <div
            className="ml-2 pl-2.5 border-l border-white/5 flex flex-col"
            style={{ gap: view.categoryGap }}
          >
            {childGroups.map((child) => renderGroup(child))}
          </div>
        )}
      </Droppable>
    );
  };

  return (
    <div
      className="w-full h-full flex flex-col text-slate-100 overflow-hidden relative font-sans"
      onDragOver={(e) => handleHtml5DragOver(e)}
      onDragLeave={handleHtml5DragLeave}
      onDrop={(e) => handleHtml5Drop(e)}
    >
      {/* Drag Over Overlay Toast / Indicator */}
      {isDragOver && (
        <LauncherDropIndicator
          currentDragTargetName={currentDragTargetName}
        />
      )}

      {/* Top Header Bar */}
      <div className="h-10 border-b border-white/5 px-4 flex items-center justify-between bg-black/20 flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm">{activeTopCategory?.data.icon || "📁"}</span>
          <span className="text-body font-bold text-slate-200 tracking-wide">
            {activeTopCategory?.name || t("launcher.quickLaunch")}
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
            className={`px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer ${
              isSearchOpen
                ? "ui-selected"
                : "ui-btn"
            }`}             title={t("launcher.searchAll")}
          >
            <Search className="w-3 h-3" />
            <span className="text-caption">{t("launcher.search")}</span>
          </button>

          {/* 全盘文件搜索悬浮窗（Everything 式，索引管理入口） */}
          <button
            onClick={() => {
              invoke("open_file_search_popup").catch(() => {});
            }}
            className="ui-btn px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer"
            title={t("launcher.fileSearchTitle")}
          >
            <Database className="w-3 h-3" />
            <span className="text-caption">{t("launcher.fileSearch")}</span>
          </button>

          {/* 浏览器收藏夹导入已删除：该能力现在归收藏模块（favorites） */}

          {/* 检测 */}
          <LauncherCheckButton
            runCheck={runCheck}
            checking={checking}
            allItems={allItems}
            missingCount={missingCount}
          />

          {/* 清理无效项目（检测为不存在） */}
          <LauncherCleanupButton
            cleanupInvalidItems={cleanupInvalidItems}
            checking={checking}
            missingCount={missingCount}
            allItems={allItems}
          />

          {/* 收缩/展开全部子分组 */}
          <LauncherCollapseButton
            handleToggleCollapseAll={handleToggleCollapseAll}
            activeSubIds={activeSubIds}
            isAllCollapsed={isAllCollapsed}
          />

          {/* 使用统计 */}
          <button
            onClick={() => setUsageStatsOpen((v) => !v)}
            className={`px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer ${
              usageStatsOpen
                ? "ui-selected"
                : "ui-btn"
            }`}
            title={t("launcher.usageStatsTitle")}
          >
            <BarChart3 className="w-3 h-3 text-amber-400" />
            <span className="text-caption">{t("launcher.usageStats")}</span>
          </button>

          {/* 视图设置 */}
          <LauncherViewSettings
            viewSettingsRef={viewSettingsRef}
            setViewSettingsOpen={setViewSettingsOpen}
            viewSettingsOpen={viewSettingsOpen}
            settings={settings}
            saveViewSettings={saveViewSettings}
          />

          {/* 关联文件夹刷新 */}
          {isFolderCategory && (
            <button
              onClick={() => setFolderReloadKey((k) => k + 1)}
              disabled={folderLoading || !activeFolderPath}
              className="px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border ui-btn transition cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
              title={t("launcher.folderRefreshTitle")}
            >
              <RefreshCw className={`w-3.5 h-3.5 text-cyan-400 ${folderLoading ? "animate-spin" : ""}`} />
            </button>
          )}

          {/* 备份导出 / 导入（图标按钮，避免顶栏拥挤） */}
          <button
            onClick={handleExportBackup}
            disabled={backupBusy}
            className="p-1.5 rounded-ctl border ui-btn transition cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            title={t("launcher.exportBackupTitle")}
          >
            <Download className="w-3.5 h-3.5 text-emerald-400" />
          </button>
          <button
            onClick={handleImportBackup}
            disabled={backupBusy}
            className="p-1.5 rounded-ctl border ui-btn transition cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            title={t("launcher.importBackupTitle")}
          >
            <Upload className="w-3.5 h-3.5 text-cyan-400" />
          </button>

          {/* Add Item（聚合分类只读，不提供手动添加） */}
          {!isAggregateCategory && (
            <button
              onClick={() => {
                setEditingItem(null);
                setTargetClassificationId(activeTopCategory ? activeTopCategory.id : 1);
                setItemModalOpen(true);
              }}
              className="px-3 py-1 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white rounded-ctl text-body font-medium transition cursor-pointer shadow-md shadow-[var(--module-accent-ring)] flex items-center gap-1"
            >
              <Plus className="w-3 h-3" />
              <span className="text-caption">{t("launcher.addItem")}</span>
            </button>
          )}
        </div>
      </div>

      {/* Kira 贴心问候（生命力）：头像 + 时段开场白 + 轮换问候 */}
      <div className="flex items-center gap-2.5 px-4 py-1.5 bg-[var(--module-accent)]/5 border-b border-white/5 flex-shrink-0">
        <VexGlowAvatar size={26} />
        <span className="text-caption text-slate-300 truncate">
          <VexGreeting />
        </span>
      </div>

      {/* 检测进度条（Kira 忙碌小助手） */}
      {checkProgress && (
        <LauncherCheckProgress
          checkProgress={checkProgress}
          checking={checking}
          stopCheck={stopCheck}
        />
      )}

      {/* Main Split: Left Vertical Categories (128px) | Right Cards Flow */}
      <DndContext
        sensors={sensors}
        collisionDetection={customCollisionDetection}
        onDragStart={handleDragStart}
        onDragOver={handleDragOver}
        onDragEnd={handleDragEnd}
        onDragCancel={handleDragCancel}
      >
      <div className="flex-1 flex min-h-0 relative">
        {/* Left Vertical Categories (Fixed 128px) */}
        <LauncherCategorySidebar
          topCategories={topCategories}
          activeTopCategory={activeTopCategory}
          justDraggedAtRef={justDraggedAtRef}
          setActiveParentId={setActiveParentId}
          isSearchOpen={isSearchOpen}
          closeSearch={closeSearch}
          setIsDragOver={setIsDragOver}
          setTargetDragSubId={setTargetDragSubId}
          processDroppedPaths={processDroppedPaths}
          setCategoryContextMenu={setCategoryContextMenu}
          targetDragSubId={targetDragSubId}
          setEditingCategory={setEditingCategory}
          setCategoryParentId={setCategoryParentId}
          setCategoryModalOpen={setCategoryModalOpen}
        />

        {/* Right Groups / Cards Stream */}
        <div
          className="flex-1 overflow-y-auto p-5 bg-white/[0.01] flex flex-col"
          style={{ gap: view.categoryGap }}
        >
          {/* 聚合分类：跨分类聚合项目（只读，不支持拖动排序） */}
          {isAggregateCategory ? (
            <LauncherAggregateItems
              activeTopCategory={activeTopCategory}
              aggregateItems={aggregateItems}
              gridClass={gridClass}
              densityGap={densityGap}
              gridStyle={gridStyle}
              itemView={itemView}
              checkResults={checkResults}
              view={view}
              handleExecuteItem={handleExecuteItem}
              setItemContextMenu={setItemContextMenu}
            />
          ) : (
          <>
          {/* 1. If Category has Sub-Categories (recursively rendered) */}
          {subCategories.length > 0 ? (
            subCategories.map((sub) => renderGroup(sub))
          ) : null}

          {/* 关联文件夹分类：实时镜像目录内容（虚拟项目，不落库） */}
          {isFolderCategory && (
            <LauncherFolderItems
              appearance={appearance}
              activeFolderPath={activeFolderPath}
              folderItems={folderItems}
              folderLoading={folderLoading}
              folderError={folderError}
              setEditingCategory={setEditingCategory}
              activeTopCategory={activeTopCategory}
              setCategoryParentId={setCategoryParentId}
              setCategoryModalOpen={setCategoryModalOpen}
              folderVirtualItems={folderVirtualItems}
              launchVirtual={launchVirtual}
              setFolderContextMenu={setFolderContextMenu}
            />
          )}

          {/* 2. Direct items under active category */}
          {(() => {
            const directItems = activeTopCategory
              ? itemsByClassification.get(activeTopCategory.id) || []
              : [];
            if (directItems.length === 0 && subCategories.length > 0) return null;

            return (
              <Droppable
                id={activeTopCategory ? `cat:${activeTopCategory.id}` : `cat:-1`}
                dataCatId={activeTopCategory ? activeTopCategory.id : undefined}
                className={`space-y-2.5 rounded-card transition ${
                  targetDragSubId === activeTopCategory?.id
                    ? "bg-[var(--module-accent-soft)] ring-1 ring-[var(--module-accent-ring)]"
                    : ""
                }`}
                onDragOver={(e) =>
                  activeTopCategory
                    ? handleHtml5DragOver(e, activeTopCategory.id)
                    : handleHtml5DragOver(e)
                }
                onDrop={(e) =>
                  activeTopCategory
                    ? handleHtml5Drop(e, activeTopCategory.id)
                    : handleHtml5Drop(e)
                }
              >
                {subCategories.length > 0 && (
                  <div className="flex items-center justify-start relative py-0.5">
                    <div className="absolute inset-0 flex items-center pointer-events-none">
                      <div className="w-full border-t border-white/5" />
                    </div>
                    <div className="relative bg-white/[0.04] backdrop-blur-sm pr-4 pl-0.5 font-semibold text-slate-400 tracking-wide flex items-center gap-1.5" style={categoryNameStyle}>
                      <span className="leading-none" style={{ fontSize: view.categoryFontSize + 2 }}>
                        {activeTopCategory?.data?.icon || "📁"}
                      </span>
                      <span>{t("launcher.otherItems")}</span>
                      {directItems.length > 0 && (
                        <span className="text-tiny text-slate-600 font-normal">
                          {directItems.length}
                        </span>
                      )}
                    </div>
                  </div>
                )}

                {directItems.length > 0 ? (
                  <SortableContext
                    items={directItems.map((i) => `item:${i.id}`)}
                    strategy={horizontalListSortingStrategy}
                  >
                    <div className={`${gridClass} ${densityGap}`} style={gridStyle}>
                      {directItems.map((item) => (
                        <SortableItem
                          key={item.id}
                          item={item}
                          view={itemView}
                          checkResults={checkResults}
                          onClick={() => {
                            if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发启动
                            handleExecuteItem(item);
                          }}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                          }}
                        />
                      ))}
                    </div>
                  </SortableContext>
                ) : subCategories.length === 0 && !isFolderCategory ? (
                  <VexEmptyState
                    title={t("launcher.emptyCategory")}
                    desc={
                      <span className="inline-block max-w-sm">
                        {t("launcher.emptyCategoryHint")}{" "}
                        <span className="text-[var(--module-accent)] font-medium">
                          {t("launcher.dragHere")}
                        </span>
                      </span>
                    }
                    tick={t("launcher.emptyCategoryTick")}
                    avatarSize={52}
                    className="!py-20"
                    action={{
                      label: t("launcher.addItem"),
                      onClick: () => {
                        if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发
                        setEditingItem(null);
                        setTargetClassificationId(
                          activeTopCategory ? activeTopCategory.id : 1
                        );
                        setItemModalOpen(true);
                      },
                    }}
                    secondaryAction={{
                      label: t("launcher.newSubCategory"),
                      onClick: () => {
                        if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发
                        setEditingCategory(null);
                        setCategoryParentId(
                          activeTopCategory ? activeTopCategory.id : null
                        );
                        setCategoryModalOpen(true);
                      },
                    }}
                  />
                ) : null}
              </Droppable>
            );
          })()}
          </>
          )}
        </div>

        {usageStatsOpen && (
          <LauncherUsageStats
            setUsageStatsOpen={setUsageStatsOpen}
            usageStats={usageStats}
            classificationMap={classificationMap}
            setActiveParentId={setActiveParentId}
            activeParentId={activeParentId}
            handleExecuteItem={handleExecuteItem}
          />
        )}

        {/* Figure 2: Unified Search Overlay */}
        {isSearchOpen && (
          <LauncherSearchOverlay
            searchInputRef={searchInputRef}
            searchQuery={searchQuery}
            setSearchQuery={setSearchQuery}
            setSearchSelectedIndex={setSearchSelectedIndex}
            searchTotal={searchTotal}
            searchSelectedIndex={searchSelectedIndex}
            searchQuickActions={searchQuickActions}
            launchVirtual={launchVirtual}
            closeSearch={closeSearch}
            searchResults={searchResults}
            handleExecuteItem={handleExecuteItem}
            effectiveFileHits={effectiveFileHits}
            openFileHit={openFileHit}
            webSources={webSources}
            classificationMap={classificationMap}
          />
      )}
      </div>

      {/* DragOverlay：拖拽时的悬浮卡片（跟随光标，自动让位由 SortableContext 处理） */}
      <DragOverlay dropAnimation={null}>
        {activeDragItem && (
          <div className={cardClass(activeDragItem, itemView, checkResults)} style={{ borderRadius: itemView.itemRadius }}>
            <ItemCardBody item={activeDragItem} view={itemView} checkResults={checkResults} />
          </div>
        )}
      </DragOverlay>
      </DndContext>

      {/* Item Context Menu */}
      {itemContextMenu && (
        <LauncherItemMenu
          itemContextMenu={itemContextMenu}
          handleExecuteItem={handleExecuteItem}
          setItemContextMenu={setItemContextMenu}
          showToast={showToast}
          setEditingItem={setEditingItem}
          setTargetClassificationId={setTargetClassificationId}
          setItemModalOpen={setItemModalOpen}
          handleDeleteItem={handleDeleteItem}
        />
      )}

      {/* 关联文件夹条目右键菜单 */}
      {folderContextMenu && (
        <LauncherFolderMenu
          folderContextMenu={folderContextMenu}
          launchVirtual={launchVirtual}
          setFolderContextMenu={setFolderContextMenu}
          showToast={showToast}
          addFolderItemToClassification={addFolderItemToClassification}
        />
      )}

      {/* Category Context Menu */}
      {categoryContextMenu && (
        <LauncherCategoryMenu
          categoryContextMenu={categoryContextMenu}
          setEditingCategory={setEditingCategory}
          setCategoryParentId={setCategoryParentId}
          setCategoryModalOpen={setCategoryModalOpen}
          setCategoryContextMenu={setCategoryContextMenu}
          setEditingItem={setEditingItem}
          setTargetClassificationId={setTargetClassificationId}
          setItemModalOpen={setItemModalOpen}
          openMoveItemsModal={openMoveItemsModal}
          handleDeleteCategory={handleDeleteCategory}
        />
      )}

      {/* 批量转移分类项目弹窗 */}
      {moveItemsModalOpen && moveItemsSource && (
        <LauncherMoveCategoriesDialog
          moveItemsLoading={moveItemsLoading}
          setMoveItemsModalOpen={setMoveItemsModalOpen}
          moveItemsSource={moveItemsSource}
          moveItemsSourceCount={moveItemsSourceCount}
          moveItemsTargetList={moveItemsTargetList}
          classifications={classifications}
          moveItemsTarget={moveItemsTarget}
          setMoveItemsTarget={setMoveItemsTarget}
          handleMoveItemsConfirm={handleMoveItemsConfirm}
        />
      )}

      {/* Toast Notification */}
      {toastMsg && (
        <div className="fixed bottom-6 right-6 z-[300] bg-[var(--module-accent)] text-white text-body px-4 py-2.5 rounded-card shadow-[0_0_14px_color-mix(in_srgb,var(--module-accent)_48%,transparent),0_0_36px_color-mix(in_srgb,var(--module-accent)_26%,transparent)] flex items-center gap-2 animate-in fade-in slide-in-from-bottom-2 duration-150">
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

      {/* 删除分类确认弹框：必须先决定「子分类与项目」的去向，避免留下孤儿数据 */}
      {pendingDeleteCategory && (
        <LauncherDeleteCategoryDialog
          pendingDeleteCategory={pendingDeleteCategory}
          scopeOf={scopeOf}
          deleteCascade={deleteCascade}
          setDeleteCascade={setDeleteCascade}
          classifications={classifications}
          deleteTarget={deleteTarget}
          setDeleteTarget={setDeleteTarget}
          setDeleteNewName={setDeleteNewName}
          deleteNewName={deleteNewName}
          deletingCategory={deletingCategory}
          setPendingDeleteCategory={setPendingDeleteCategory}
          confirmDeleteCategory={confirmDeleteCategory}
        />
      )}
    </div>
  );
}
