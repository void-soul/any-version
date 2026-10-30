import type { UIEvent } from "react";
import type { CollabRoom } from "../../contracts/ai";
import { useTranslation } from "react-i18next";
import VexEmptyState from "../VexEmptyState";
import { MessagesSquare, Trash2, Loader2 } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";

interface CollabRoomListProps {
  onRoomsScroll: (e: UIEvent<HTMLDivElement>) => void;
  rooms: CollabRoom[];
  loadingMore: boolean;
  activeRoom: CollabRoom | null;
  setActiveRoom: Dispatch<SetStateAction<CollabRoom | null>>;
  deleteRoom: (id: string) => Promise<void>;
  hasMore: boolean;
}

/** Pure CollabRoomList view; its owning panel retains side effects and state. */
export function CollabRoomList({ onRoomsScroll, rooms, loadingMore, activeRoom, setActiveRoom, deleteRoom, hasMore }: CollabRoomListProps) {
  const { t } = useTranslation();
  return (
    <div className="flex-1 overflow-y-auto space-y-0.5" onScroll={onRoomsScroll}>
      {rooms.length === 0 && !loadingMore && (
        <VexEmptyState
          title={t("collab.noSessionTitle")}
          desc={t("collab.noSessionDesc")}
          tick={t("collab.noSessionTick")}
          avatarSize={34}
          className="!py-8"
        />
      )}
      {rooms.map((r) => (
        <div
          key={r.id}
          className={`group flex items-center gap-1 px-2 py-1.5 rounded-ctl cursor-pointer text-caption ${
            activeRoom?.id === r.id
              ? "bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] text-[var(--module-accent)]"
              : "text-slate-400 hover:bg-white/5"
          }`}
          onClick={() => setActiveRoom(r)}
        >
          <MessagesSquare className="w-3 h-3 flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <div className="truncate font-medium">{r.name}</div>
            <div className="truncate text-[8px] text-slate-500">{r.project_path}</div>
          </div>
          <button
            onClick={(e) => {
              e.stopPropagation();
              deleteRoom(r.id);
            }}
            className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-red-500/20 text-slate-500 hover:text-red-300 cursor-pointer"
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
      ))}
      {loadingMore && (
        <div className="flex items-center justify-center gap-1.5 py-2 text-micro text-slate-500">
          <Loader2 className="w-3 h-3 animate-spin" /> {t("collab.loading")}
        </div>
      )}
      {!hasMore && rooms.length > 0 && (
        <p className="text-center text-[8px] text-slate-600 py-1">{t("collab.endOfList")}</p>
      )}
    </div>
  );
}
