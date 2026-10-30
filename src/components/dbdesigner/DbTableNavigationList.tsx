import type { useHoverPreview } from "../shared/HoverPreview";
import type { DbTableNavigationRow } from "./canvasTypes";
import "@xyflow/react/dist/style.css";
import { TablePreviewBody } from "./DbDesignerPanel.TablePreviewBody";
import type { Dispatch, SetStateAction } from "react";

interface DbTableNavigationListProps {
  hp: Pick<ReturnType<typeof useHoverPreview>, "show" | "scheduleHide" | "hideNow" | "preview">;
  tableTreeRows: DbTableNavigationRow[];
  setHighlightId: Dispatch<SetStateAction<string | null>>;
  setSelectedId: Dispatch<SetStateAction<string | null>>;
  setCenterTargetId: Dispatch<SetStateAction<string | null>>;
  setCenterSignal: Dispatch<SetStateAction<number>>;
  highlightId: string | null;
}

/** Pure DbTableNavigationList view; its owning panel retains side effects and state. */
export function DbTableNavigationList({ hp, tableTreeRows, setHighlightId, setSelectedId, setCenterTargetId, setCenterSignal, highlightId }: DbTableNavigationListProps) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-1 py-1" onScroll={() => hp.hideNow()}>
      {tableTreeRows.map((r) => (
        <button
          key={r.id}
          onClick={() => {
            // 导航点表名 = 「我要看这张表」：高亮 + 画布移过去 + 打开面板。
            // 三个都做，因为它本来就是唯一带「定位」的入口（画布上找一张表全靠拖）。
            setHighlightId(r.id);
            setSelectedId(r.id);
            setCenterTargetId(r.id);
            setCenterSignal((s) => s + 1);
          }}
          className={`flex w-full items-center gap-1 rounded py-1 pr-1.5 text-left text-tiny cursor-pointer transition-colors ${
            r.id === highlightId
              ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
              : r.related
                ? "text-cyan-300 hover:bg-white/5"
                : "text-slate-400 hover:bg-white/5 hover:text-slate-200"
          }`}
          style={{ paddingLeft: 6 + r.depth * 10 }}
          // 悬浮展示完整字段（与思维导图的节点预览共用 shared/HoverPreview）。
          //
          // **锚点用行的右缘 + 顶边，不是鼠标坐标**：跟随鼠标时每一行弹出的位置
          // 都不同，眼睛要重新找；按列对齐后所有预览的左边框落在同一条竖线上，
          // 上下扫过列表时视线不用横向漂移（用户要求）。
          // 原生 title 只能塞一行字，放不下「这张表有哪些字段」—— 而表列表
          // 刻意做成一行一个（字段最多、名字最短），不给悬浮就只能放大画布去看，
          // 那是另一件事。onMouseMove 只在换行时重建内容，避免逐帧 setState。
          onMouseEnter={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            hp.show({
              title: r.name,
              x: rect.right,
              y: rect.top,
              accent: /^#[0-9a-f]{6}$/i.test(r.color ?? "") ? r.color : undefined,
              content: <TablePreviewBody row={r} />,
            });
          }}
          onMouseMove={(e) => {
            if (hp.preview?.title !== r.name) {
              const rect = e.currentTarget.getBoundingClientRect();
              hp.show({
                title: r.name, x: rect.right, y: rect.top,
                accent: /^#[0-9a-f]{6}$/i.test(r.color ?? "") ? r.color : undefined,
                content: <TablePreviewBody row={r} />,
              });
            }
          }}
          onMouseLeave={() => hp.scheduleHide()}
        >
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1">
              {/* 与画布同源的配色，一眼分出系列 */}
              <span
                className="w-1.5 h-1.5 rounded-full flex-shrink-0"
                style={{ background: /^#[0-9a-f]{6}$/i.test(r.color ?? "") ? r.color : "var(--module-accent)" }}
              />
              <span className="truncate">{r.name}</span>
              {r.relCount > 0 ? <span className="ml-auto text-micro text-slate-600">{r.relCount}</span> : null}
            </span>
            {/* 表注释：AI 产出里最值钱的部分，光看表名猜不出它存什么。
                窄侧栏里必须 truncate —— 否则一行注释就能把整栏撑宽 */}
            {r.comment ? (
              <span className="block truncate text-[9px] leading-tight text-slate-600" title={r.comment}>
                {r.comment}
              </span>
            ) : null}
          </span>
        </button>
      ))}
    </div>
  );
}
