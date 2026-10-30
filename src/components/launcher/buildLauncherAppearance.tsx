import type { LauncherSetting } from "./types";
import { type ItemView } from "./LauncherPanel.model";

/** Owns the buildLauncherAppearance feature, independently of page rendering. */
export function buildLauncherAppearance({ settings }: {
  settings: LauncherSetting;
}) {


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

  // 卡片边框（受全局设置控制）
  const cardBorderClass = view.itemBorder
    ? ""
    : "!border-transparent";

  const itemNameStyle: React.CSSProperties = {
    fontSize: view.itemFontSize,
  };

  const categoryNameStyle: React.CSSProperties = {
    fontSize: view.categoryFontSize,
  };


  // 卡片视图参数（SortableItem / DragOverlay 共用）
  const itemView: ItemView = {
    iconSize: view.iconSize,
    showName: view.showName,
    iconBg: view.iconBg,
    itemFontSize: view.itemFontSize,
    itemRadius: view.itemRadius,
    itemBorder: view.itemBorder,
    densityCard,
    cardBorderClass,
    itemNameStyle,
  };
  return {
    view,
    gridStyle,
    gridClass,
    densityGap,
    categoryNameStyle,
    itemView,
  };
}

export type LauncherAppearance = ReturnType<typeof buildLauncherAppearance>;
