import { useTranslation } from "react-i18next";
import type { Dispatch, SetStateAction } from "react";

interface CollabRoomCreateProps {
  newName: string;
  setNewName: Dispatch<SetStateAction<string>>;
  handlePickProject: () => Promise<void>;
  newProject: string;
  createRoom: () => Promise<void>;
  setCreating: Dispatch<SetStateAction<boolean>>;
}

/** Pure CollabRoomCreate view; its owning panel retains side effects and state. */
export function CollabRoomCreate({ newName, setNewName, handlePickProject, newProject, createRoom, setCreating }: CollabRoomCreateProps) {
  const { t } = useTranslation();
  return (
    <div className="mb-2 p-2 rounded-ctl bg-slate-900/60 border border-white/10 space-y-1.5">
      <input
        value={newName}
        onChange={(e) => setNewName(e.target.value)}
        placeholder={t("collab.sessionNamePh")}
        className="w-full bg-slate-800 border border-white/10 rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
      />
      <button
        onClick={handlePickProject}
        className={`w-full px-2 py-1 rounded text-left text-tiny border border-white/10 truncate cursor-pointer ${
          newProject
            ? "bg-slate-800 text-slate-200"
            : "bg-slate-800/60 text-slate-500 hover:text-slate-300"
        }`}
        title={newProject || t("collab.pickProjectDir")}
      >
        {newProject || `${t("collab.pickProjectDir")}…`}
      </button>
      <div className="flex gap-1">
        <button
          onClick={createRoom}
          className="flex-1 px-2 py-1 rounded bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-tiny font-semibold cursor-pointer"
        >
          {t("collab.create")}
        </button>
        <button
          onClick={() => setCreating(false)}
          className="px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-tiny cursor-pointer"
        >
          {t("collab.cancel")}
        </button>
      </div>
    </div>
  );
}
