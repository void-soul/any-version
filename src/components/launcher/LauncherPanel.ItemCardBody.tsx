import { Folder, Globe, FileText } from "lucide-react";
import { Item } from "./types";
import { type ItemView } from "./LauncherPanel.model";
/** 卡片内容（SortableItem 与 DragOverlay 共用，消除两处重复） */
export function ItemCardBody(props: {
  item: Item;
  view: ItemView;
  checkResults: Record<number, { exists: boolean }>;
}) {
  const { item, view, checkResults } = props;
  const missing = checkResults[item.id] && !checkResults[item.id].exists;
  // per-item 图标背景色块优先于全局设置（参考 DawnLauncher）
  const itemIconBg = item.data.iconBackgroundColor ?? view.iconBg;
  return (
    <>
      {item.data.runAsAdmin && (
        <div className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-amber-400" />
      )}
      {missing && <div className="absolute -top-1 -left-1 w-2 h-2 rounded-full bg-red-500 shadow shadow-red-500/50" />}
      <div
        className={`rounded-ctl flex items-center justify-center flex-shrink-0 group-hover:scale-105 transition overflow-hidden ${
          itemIconBg
            ? item.data.iconBackgroundColor
              ? ""
              : "bg-white/5"
            : "bg-transparent"
        }`}
        style={{
          width: view.iconSize,
          height: view.iconSize,
          backgroundColor: itemIconBg && item.data.iconBackgroundColor
            ? item.data.iconBackgroundColorValue || "#0078D7"
            : undefined,
        }}
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
        <span
          className="text-slate-200 truncate group-hover:text-[var(--module-accent)] transition min-w-0 flex-1 font-medium"
          style={view.itemNameStyle}
        >
          {item.name}
        </span>
      )}
    </>
  );
}
