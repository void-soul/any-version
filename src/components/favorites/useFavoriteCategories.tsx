import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ChevronRight, RefreshCw, Plus, X, Sparkles, Loader2, Star } from "lucide-react";
import { SharedButton } from "../shared/Button";
import { toast } from "../shared/Toast";
import { theamedConfirm, theamedPrompt } from "../shared/ThemedAlert";
import { type ApplyCategoryMovesResult, type CategoryMovePlan, type PlannedCategoryGroup, type PlannedCategoryMove, type FavoriteRow, type FavoriteStats, type FavoriteCategoryNode } from "./types";
import { type FavTask } from "./FavoritesPanel.model";
import { CategoryTree } from "./FavoritesPanel.CategoryTree";
import { CategoryCheckTree } from "./FavoritesPanel.CategoryCheckTree";

/** Owns the FavoriteCategories feature, independently of page rendering. */
export function useFavoriteCategories({ stats, refresh, providerId, modelId, running, leftWidth, starredCount }: {
  stats: FavoriteStats | null;
  refresh: () => Promise<void>;
  providerId: string;
  modelId: string;
  running: FavTask[];
  leftWidth: number;
  starredCount: number;
}) {
  const { t } = useTranslation();

  // 分类筛选：走分类 id（树），名字只用于回显
  const [categoryId, setCategoryId] = useState<number | null>(null);

  const [categoryName, setCategoryName] = useState<string | null>(null);

  // 只看星标（左侧「星标」入口）：与分类筛选可叠加 —— 同时选中时两者都要满足
  const [starredOnly, setStarredOnly] = useState(false);

  // 分类树里展开的节点
  const [expandedCats, setExpandedCats] = useState<Set<number>>(new Set());

  // 分类右键菜单
  const [catMenu, setCatMenu] = useState<{ id: number | null; name: string; x: number; y: number } | null>(null);

  // 条目分类选择器
  const [pickerFor, setPickerFor] = useState<FavoriteRow | null>(null);

  const [pickerSelected, setPickerSelected] = useState<number[]>([]);

  // 整理类目：AI 只生成「改上级」的方案，用户在预览里勾选后才落库
  const [organizeOpen, setOrganizeOpen] = useState(false);

  const [organizePlan, setOrganizePlan] = useState<CategoryMovePlan | null>(null);

  const [organizeSelected, setOrganizeSelected] = useState<number[]>([]);

  const [organizeBusy, setOrganizeBusy] = useState(false);

  const [organizeApplying, setOrganizeApplying] = useState(false);

  const [organizeError, setOrganizeError] = useState<string | null>(null);

  // ── 分类树操作（对齐启动模块：新建子分类 / 重命名 / 删除 / 同级排序）──
  const cats: FavoriteCategoryNode[] = stats?.categories || [];

  const findCat = (id: number | null): FavoriteCategoryNode | null => {
    if (id == null) return null;
    const walk = (list: FavoriteCategoryNode[]): FavoriteCategoryNode | null => {
      for (const c of list) {
        if (c.id === id) return c;
        const hit = walk(c.children);
        if (hit) return hit;
      }
      return null;
    };
    return walk(cats);
  };

  /** 按名字找分类（条目标签存的是名字，筛选要的是 id） */
  const findCatByName = (name: string): FavoriteCategoryNode | null => {
    const walk = (list: FavoriteCategoryNode[]): FavoriteCategoryNode | null => {
      for (const c of list) {
        if (c.name === name) return c;
        const hit = walk(c.children);
        if (hit) return hit;
      }
      return null;
    };
    return walk(cats);
  };

  /** 同级排序用的 id 列表（上下移动时只重排这一层） */
  const siblingsOf = (id: number | null): FavoriteCategoryNode[] => {
    if (id == null) return cats;
    const walk = (list: FavoriteCategoryNode[]): FavoriteCategoryNode[] | null => {
      for (const c of list) {
        if (c.id === id) return list;
        const hit = walk(c.children);
        if (hit) return hit;
      }
      return null;
    };
    return walk(cats) || [];
  };

  /**
   * 新建分类。
   *
   * 只保留「普通分类」一种。之前还有个 kind="follow" 的「关注分组」，已删除：
   * 它建完之后**没有任何入口能把收藏项归进去**（唯一的归类入口是铅笔按钮，
   * 而且默认 hover 才显形），于是用户建了一堆分组却发现「咋用啊」——
   * 真正在用的其实一直是星标。留一个用不上的概念比不留更费解。
   *
   * 已存在的 follow 分组**不删数据**，按普通分类照样显示与筛选（不替用户删东西）。
   */
  const createCat = async (parentId: number | null) => {
    const name = await theamedPrompt(t(pickCatKey(parentId)), {
      title: parentId == null ? t("favorites.newCategory") : t("favorites.newSubCategory"),
      confirmText: t("common.create"),
    });
    if (!name?.trim()) return;
    try {
      const id = await invoke<number>("fav_create_category", { name: name.trim(), parentId, kind: "topic" });
      if (parentId != null) setExpandedCats((s) => new Set(s).add(parentId));
      setCategoryId(id);
      setCategoryName(name.trim());
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  function pickCatKey(parentId: number | null) {
    return parentId == null ? "favorites.newCategoryPrompt" : "favorites.newSubCategoryPrompt";
  }

  const renameCat = async (id: number) => {
    const cur = findCat(id);
    const name = await theamedPrompt(t("favorites.renameCategoryPrompt"), {
      title: t("favorites.renameCategory"),
      initial: cur?.name || "",
    });
    if (!name?.trim()) return;
    try {
      await invoke("fav_rename_category", { id, name: name.trim() });
      if (categoryId === id) setCategoryName(name.trim());
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const deleteCat = async (id: number) => {
    const cur = findCat(id);
    if (!(await theamedConfirm(t("favorites.deleteCategoryConfirm", { name: cur?.name || "" }), { danger: true }))) return;
    try {
      await invoke("fav_delete_category", { id });
      if (categoryId === id) {
        setCategoryId(null);
        setCategoryName(null);
      }
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** 同级上移 / 下移：只重排这一层，动完立刻落盘 */
  const moveCat = async (id: number, dir: -1 | 1) => {
    const list = siblingsOf(id);
    const from = list.findIndex((c) => c.id === id);
    const to = from + dir;
    if (from < 0 || to < 0 || to >= list.length) return;
    const next = [...list];
    [next[from], next[to]] = [next[to], next[from]];
    try {
      await invoke("fav_reorder_categories", {
        orders: next.map((c, i) => [c.id, i]),
      });
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  // ── 整理类目（AI 只给「改上级」建议，预览确认后才落库）──
  const organizeReady = Boolean(providerId && modelId) && running.length === 0;

  /** 请求 AI 生成方案。只读：不落库，用户勾选后才应用。 */
  const loadOrganizePlan = async () => {
    if (!providerId || !modelId || organizeBusy) return;
    setOrganizeBusy(true);
    setOrganizeError(null);
    setOrganizePlan(null);
    setOrganizeSelected([]);
    try {
      const plan = await invoke<CategoryMovePlan>("fav_plan_category_moves", {
        providerId,
        modelId,
      });
      setOrganizePlan(plan);
      setOrganizeSelected(plan.moves.map((m) => m.id));
    } catch (e) {
      setOrganizeError(String(e));
    } finally {
      setOrganizeBusy(false);
    }
  };

  const openOrganize = () => {
    setOrganizeOpen(true);
    void loadOrganizePlan();
  };

  /** 只把勾选的建议发回后端；后端会重新校验一次（前端传的可以是子集）。 */
  const applyOrganize = async () => {
    if (!organizePlan || organizeApplying) return;
    const picked = new Set(organizeSelected);
    const moves = organizePlan.moves
      .filter((m: PlannedCategoryMove) => picked.has(m.id))
      .map((m) => ({ id: m.id, parentId: m.toParentId }));
    if (moves.length === 0) return;
    // 新建分组由后端按「被勾选的移动」筛选用到的，不会被多余创建
    const groups = organizePlan.groups.map((g: PlannedCategoryGroup) => ({
      tempId: g.tempId,
      name: g.name,
      parentId: g.parentId,
      parentName: g.parentName ?? null,
      depth: g.depth,
    }));
    setOrganizeApplying(true);
    try {
      const result = await invoke<ApplyCategoryMovesResult>("fav_apply_category_moves", {
        moves,
        groups,
      });
      setOrganizeOpen(false);
      setOrganizePlan(null);
      await refresh();
      if (result.skipped.length > 0) {
        toast(
          t("favorites.organizeAppliedSkipped", {
            applied: result.applied,
            skipped: result.skipped.length,
          }),
          "info",
        );
      } else if (result.created > 0) {
        toast(
          t("favorites.organizeAppliedWithGroups", {
            count: result.applied,
            groups: result.created,
          }),
          "ok",
        );
      } else {
        toast(t("favorites.organizeApplied", { count: result.applied }), "ok");
      }
    } catch (e) {
      toast(t("favorites.organizeApplyFail", { err: String(e) }), "err");
    } finally {
      setOrganizeApplying(false);
    }
  };

  const organizePickedCount =
    organizePlan?.moves.filter((m: PlannedCategoryMove) => organizeSelected.includes(m.id)).length ?? 0;

  // ── 条目分类选择器 ──
  const openPicker = (item: FavoriteRow) => {
    setPickerFor(item);
    const ids = (item.tags || [])
      .map((name) => cats.find((c) => c.name === name)?.id)
      .filter((v): v is number => typeof v === "number");
    setPickerSelected(ids);
  };

  const submitPicker = async () => {
    if (!pickerFor) return;
    try {
      await invoke("fav_set_item_categories", { id: pickerFor.id, categoryIds: pickerSelected });
      setPickerFor(null);
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const renderCategorySidebar = () => (<div
          className="shrink-0 overflow-y-auto glass-panel p-2 space-y-0.5"
          style={{ width: leftWidth }}
        >
          {/* 顶部操作：新建分类 / 整理类目。分类多时这两个入口也不该被滚到底部 */}
          <div className="flex items-center gap-1 pb-1 mb-0.5 border-b border-white/5">
            <button
              onClick={() => void createCat(null)}
              className="flex-1 min-w-0 inline-flex items-center justify-center gap-1 px-1 py-1 rounded-ctl text-tiny text-slate-400 hover:text-slate-200 hover:bg-white/5 cursor-pointer"
              title={t("favorites.newCategory")}
            >
              <Plus className="w-3 h-3 shrink-0" />
              <span className="truncate">{t("favorites.newCategory")}</span>
            </button>
            {/* 整理类目：AI 检查「该做子类却建成平级」的分类，可补建缺失父级 */}
            <button
              onClick={openOrganize}
              disabled={!organizeReady}
              title={
                !providerId || !modelId
                  ? t("favorites.organizeNeedProvider")
                  : running.length > 0
                    ? t("favorites.organizeBlocked")
                    : t("favorites.organizeHint")
              }
              className="flex-1 min-w-0 inline-flex items-center justify-center gap-1 px-1 py-1 rounded-ctl text-tiny text-slate-400 hover:text-slate-200 hover:bg-white/5 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {organizeBusy ? (
                <Loader2 className="w-3 h-3 shrink-0 animate-spin" />
              ) : (
                <Sparkles className="w-3 h-3 shrink-0" />
              )}
              <span className="truncate">{t("favorites.organize")}</span>
            </button>
          </div>

          <button
            onClick={() => { setCategoryId(null); setCategoryName(null); }}
            className={`w-full text-left px-2 py-1 rounded text-caption cursor-pointer transition-colors ${
              categoryId === null
                ? "bg-[var(--module-accent-soft)] text-white"
                : "text-slate-400 hover:bg-white/5"
            }`}
          >
            {t("favorites.allTags")}
            <span className="float-right text-slate-500">{stats?.total ?? 0}</span>
          </button>

          {/* 星标（特别关注）入口：与「全部」并列，筛选逻辑是 starred_only */}
          <button
            onClick={() => { setCategoryId(null); setCategoryName(null); setStarredOnly((v) => !v); }}
            className={`w-full text-left px-2 py-1 rounded text-caption cursor-pointer transition-colors ${
              starredOnly
                ? "bg-[var(--module-accent-soft)] text-white"
                : "text-slate-400 hover:bg-white/5"
            }`}
          >
            <Star className={`w-3 h-3 inline mr-1 ${starredOnly ? "text-amber-400" : "text-slate-500"}`} />
            {t("favorites.starred")}
            <span className="float-right text-slate-500">{starredCount}</span>
          </button>

          {/* 分类树：可折叠，右键出菜单（新建子分类 / 重命名 / 上移下移 / 删除）。
              关注分组（kind=follow）用琥珀色区分 —— 它由用户人工维护，
              AI 归类不会往里写，两者混在一起会让人分不清哪些是自动、哪些是手动。 */}
          <CategoryTree
            nodes={cats}
            depth={0}
            expanded={expandedCats}
            selectedId={categoryId}
            onToggle={(id) => setExpandedCats((s) => {
              const next = new Set(s);
              if (next.has(id)) next.delete(id); else next.add(id);
              return next;
            })}
            onSelect={(id, name) => { setCategoryId(id); setCategoryName(name); }}
            onContextMenu={(id, name, x, y) => setCatMenu({ id, name, x, y })}
          />

          {/* 原先这里还有一个「+ 关注分组」（kind=follow）。已删除：它建完之后没有任何入口
              能把收藏项归进去——真正在用的机制一直是星标（侧栏「星标」筛选 + 行内星标按钮）。
              已存在的 follow 分组不删数据，按普通分类照样显示与筛选。 */}

          {cats.length === 0 && (
            <p className="text-tiny text-slate-500 px-2 py-1 leading-snug">
              {t("favorites.noTagsHint")}
            </p>
          )}
        </div>);

const renderCategoryMenu = () => catMenu && (<div
          className="fixed z-[300] bg-surface-panel border border-white/15 rounded-ctl shadow-2xl py-1 text-caption min-w-[140px]"
          style={{ left: catMenu.x, top: catMenu.y }}
          onMouseLeave={() => setCatMenu(null)}
        >
          <div className="px-2 py-1 text-slate-500 truncate border-b border-white/5 mb-1">
            {catMenu.name}
          </div>
          <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
            onClick={() => { const id = catMenu.id; setCatMenu(null); void createCat(id); }}>
            {t("favorites.newSubCategory")}
          </button>
          {catMenu.id != null && (
            <>
              <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void renameCat(id); }}>
                {t("favorites.renameCategory")}
              </button>
              <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void moveCat(id, -1); }}>
                {t("favorites.moveUp")}
              </button>
              <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void moveCat(id, 1); }}>
                {t("favorites.moveDown")}
              </button>
              <button className="w-full text-left px-2 py-1 hover:bg-rose-500/15 text-rose-300 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void deleteCat(id); }}>
                {t("favorites.deleteCategory")}
              </button>
            </>
          )}
        </div>);

const renderOrganizeDialog = () => (<div className="w-full max-w-xl bg-surface-panel border border-white/15 rounded-panel p-4 shadow-2xl space-y-3 text-body">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-bold text-white">{t("favorites.organizeTitle")}</h3>
              <button
                onClick={() => { setOrganizeOpen(false); setOrganizePlan(null); }}
                disabled={organizeApplying}
                className="text-slate-400 hover:text-white p-1 cursor-pointer disabled:opacity-40"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <p className="text-tiny text-slate-500 leading-relaxed">{t("favorites.organizeDesc")}</p>

            {organizeBusy ? (
              <div className="flex items-center justify-center gap-2 py-8 text-caption text-slate-400">
                <Loader2 className="w-4 h-4 animate-spin" />
                {t("favorites.organizeRunning")}
              </div>
            ) : organizeError ? (
              <div className="py-4 text-caption text-rose-400 break-all">
                {t("favorites.organizePlanFail", { err: organizeError })}
              </div>
            ) : organizePlan ? (
              <>
                <div className="flex items-center gap-2 text-tiny text-slate-500">
                  <span>{t("favorites.organizeConsidered", { count: organizePlan.considered })}</span>
                  {organizePlan.moves.length > 0 && (
                    <>
                      <span className="text-slate-600">·</span>
                      <button
                        onClick={() => setOrganizeSelected(organizePlan.moves.map((m) => m.id))}
                        className="hover:text-slate-200 cursor-pointer"
                      >
                        {t("favorites.organizeSelectAll")}
                      </button>
                      <button
                        onClick={() => setOrganizeSelected([])}
                        className="hover:text-slate-200 cursor-pointer"
                      >
                        {t("favorites.organizeSelectNone")}
                      </button>
                    </>
                  )}
                </div>

                {organizePlan.groups.length > 0 && (
                  <div className="rounded-ctl border border-[var(--module-accent)]/20 bg-[var(--module-accent)]/5 px-2 py-1.5">
                    <div className="text-tiny text-slate-300">
                      {t("favorites.organizeNewGroups", { count: organizePlan.groups.length })}
                    </div>
                    <div className="mt-1 flex flex-wrap gap-1">
                      {organizePlan.groups.map((g: PlannedCategoryGroup) => (
                        <span
                          key={g.tempId}
                          className="text-micro px-1.5 py-0.5 rounded-full bg-white/5 text-slate-300"
                        >
                          {g.name}
                          {" · "}
                          {g.parentName
                            ? t("favorites.organizeUnder", { parent: g.parentName })
                            : t("favorites.organizeRoot")}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {organizePlan.moves.length === 0 ? (
                  <p className="py-6 text-center text-caption text-slate-400">
                    {organizePlan.skipped.length > 0
                      ? t("favorites.organizeEmptyAllSkipped")
                      : t("favorites.organizeEmpty")}
                  </p>
                ) : (
                  <div className="max-h-80 overflow-y-auto space-y-1">
                    {organizePlan.moves.map((m: PlannedCategoryMove) => (
                      <label
                        key={m.id}
                        className="flex items-start gap-2 rounded-ctl border border-white/5 bg-white/[0.02] px-2 py-1.5 hover:bg-white/5 cursor-pointer"
                      >
                        <input
                          type="checkbox"
                          checked={organizeSelected.includes(m.id)}
                          onChange={() =>
                            setOrganizeSelected((s) =>
                              s.includes(m.id) ? s.filter((x) => x !== m.id) : [...s, m.id],
                            )
                          }
                          className="mt-0.5 accent-[var(--module-accent)] cursor-pointer"
                        />
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="text-caption font-semibold text-slate-200">{m.name}</span>
                            <span className="text-micro text-slate-500">
                              {m.fromParentName ?? t("favorites.organizeRoot")}
                            </span>
                            <ChevronRight className="w-3 h-3 text-slate-600" />
                            <span className="text-micro text-[var(--module-accent)]">
                              {m.toParentName ?? t("favorites.organizeRoot")}
                            </span>
                            <span className="text-micro text-slate-600">
                              · {t("favorites.organizeDepth", { depth: m.depth })}
                            </span>
                          </div>
                          <div className="text-micro text-slate-500 truncate" title={m.path}>
                            {m.path}
                          </div>
                          {m.reason && (
                            <div className="text-micro text-slate-400 mt-0.5 leading-snug">{m.reason}</div>
                          )}
                        </div>
                      </label>
                    ))}
                  </div>
                )}

                {organizePlan.skipped.length > 0 && (
                  <details className="rounded-ctl border border-amber-500/20 bg-amber-500/5 px-2 py-1.5">
                    <summary className="text-tiny text-amber-300/90 cursor-pointer select-none">
                      {t("favorites.organizeSkipped", { count: organizePlan.skipped.length })}
                    </summary>
                    <div className="mt-1 max-h-28 overflow-y-auto space-y-0.5">
                      {organizePlan.skipped.map((s) => (
                        <div key={`${s.id}-${s.problem}`} className="text-micro text-slate-400">
                          <span className="text-slate-300">{s.name || `#${s.id}`}</span> · {s.problem}
                        </div>
                      ))}
                    </div>
                  </details>
                )}
              </>
            ) : null}

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                onClick={() => void loadOrganizePlan()}
                disabled={organizeBusy || organizeApplying}
                className="px-2.5 py-1.5 rounded-ctl border border-white/10 text-tiny text-slate-300 hover:text-white cursor-pointer disabled:opacity-40 flex items-center gap-1"
              >
                <RefreshCw className={`w-3 h-3 ${organizeBusy ? "animate-spin" : ""}`} />
                {t("favorites.organizeRegenerate")}
              </button>
              <button
                onClick={() => { setOrganizeOpen(false); setOrganizePlan(null); }}
                disabled={organizeApplying}
                className="px-2.5 py-1.5 rounded-ctl border border-white/10 text-tiny text-slate-300 hover:text-white cursor-pointer disabled:opacity-40"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={() => void applyOrganize()}
                disabled={organizePickedCount === 0 || organizeBusy || organizeApplying}
                className="px-2.5 py-1.5 rounded-ctl bg-[var(--module-accent)] text-white text-tiny font-semibold cursor-pointer disabled:opacity-40 flex items-center gap-1"
              >
                {organizeApplying && <Loader2 className="w-3 h-3 animate-spin" />}
                {t("favorites.organizeApply", { count: organizePickedCount })}
              </button>
            </div>
          </div>);

const renderCategoryPicker = () => pickerFor && (<div className="w-full max-w-sm bg-surface-panel border border-white/15 rounded-panel p-4 shadow-2xl space-y-3 text-body">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-bold text-white">{t("favorites.pickCategoryTitle")}</h3>
              <button onClick={() => setPickerFor(null)} className="text-slate-400 hover:text-white p-1">
                <X className="w-4 h-4" />
              </button>
            </div>
            <p className="text-tiny text-slate-500 truncate">{pickerFor.title}</p>
            <div className="max-h-64 overflow-y-auto space-y-0.5">
              {cats.length === 0 && (
                <p className="text-caption text-slate-500">{t("favorites.noTagsHint")}</p>
              )}
              <CategoryCheckTree
                nodes={cats}
                depth={0}
                selected={pickerSelected}
                onToggle={(id) => setPickerSelected((s) =>
                  s.includes(id) ? s.filter((x) => x !== id) : [...s, id]
                )}
              />
            </div>
            <div className="flex items-center gap-2 pt-1">
              <SharedButton className="!h-7 !px-3 !text-caption" onClick={() => void submitPicker()}>
                {t("common.save")}
              </SharedButton>
              <button onClick={() => setPickerFor(null)}
                className="px-3 py-1 rounded-ctl bg-white/5 hover:bg-white/10 text-caption text-slate-300 cursor-pointer">
                {t("common.cancel")}
              </button>
            </div>
          </div>);
  return {
    categoryId,
    setCategoryId,
    categoryName,
    setCategoryName,
    starredOnly,
    catMenu,
    pickerFor,
    organizeOpen,
    findCatByName,
    openPicker,
    renderCategorySidebar,
    renderCategoryMenu,
    renderOrganizeDialog,
    renderCategoryPicker,
  };
}
