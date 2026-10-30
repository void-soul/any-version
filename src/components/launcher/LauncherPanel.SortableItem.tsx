import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "react-i18next";
import { Item } from "./types";
import { cardClass, type ItemView } from "./LauncherPanel.model";
import { ItemCardBody } from "./LauncherPanel.ItemCardBody";
/** 可排序卡片（dnd-kit） */
export function SortableItem(props: {
  item: Item;
  view: ItemView;
  checkResults: Record<number, { exists: boolean }>;
  onClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const { item, view, checkResults } = props;
  const { t } = useTranslation();
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: `item:${item.id}`,
    data: { type: "item", itemId: item.id },
  });
  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    borderRadius: view.itemRadius,
  };
  const missing = checkResults[item.id] && !checkResults[item.id].exists;
  return (
    <div
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...listeners}
      onClick={props.onClick}
      onContextMenu={props.onContextMenu}
      className={cardClass(item, view, checkResults, isDragging ? "opacity-40" : "")}
      title={missing ? `${item.name}${t("launcher.missingSuffix")}` : item.name}
    >
      <ItemCardBody item={item} view={view} checkResults={checkResults} />
    </div>
  );
}
