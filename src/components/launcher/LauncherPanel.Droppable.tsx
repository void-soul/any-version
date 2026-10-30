import { useDroppable } from "@dnd-kit/core";
// ---------- dnd-kit 辅助组件（模块级，避免渲染期内重新挂载破坏排序动画） ----------

/** 通用可放置容器：向 dnd-kit 注册（内部项目拖拽），同时保留 HTML5 外部文件拖放入口 */
export function Droppable(props: {
  id: string;
  dataCatId?: number;
  className?: string;
  style?: React.CSSProperties;
  onClick?: React.MouseEventHandler<HTMLDivElement>;
  onDragOver?: React.DragEventHandler<HTMLDivElement>;
  onDrop?: React.DragEventHandler<HTMLDivElement>;
  onContextMenu?: React.MouseEventHandler<HTMLDivElement>;
  children?: React.ReactNode;
}) {
  const { setNodeRef } = useDroppable({ id: props.id });
  return (
    <div
      ref={setNodeRef}
      data-cat-id={props.dataCatId}
      className={props.className}
      style={props.style}
      onClick={props.onClick}
      onDragOver={props.onDragOver}
      onDrop={props.onDrop}
      onContextMenu={props.onContextMenu}
    >
      {props.children}
    </div>
  );
}
