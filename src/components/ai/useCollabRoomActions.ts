import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import type { CollabRoom as CollabRoomT } from "../../contracts/ai";
import { theamedAlert } from "../shared/ThemedAlert";
import type { Dispatch, SetStateAction } from "react";

/** Owns the CollabRoomActions feature, independently of page rendering. */
export function useCollabRoomActions({ setNewProject, newName, setNewName, newProject, refreshRooms, setActiveRoom, setCreating, activeRoom }: {
  setNewProject: Dispatch<SetStateAction<string>>;
  newName: string;
  setNewName: Dispatch<SetStateAction<string>>;
  newProject: string;
  refreshRooms: () => void;
  setActiveRoom: Dispatch<SetStateAction<CollabRoomT | null>>;
  setCreating: Dispatch<SetStateAction<boolean>>;
  activeRoom: CollabRoomT | null;
}) {
  const { t } = useTranslation();

  const handlePickProject = async () => {
    try {
      const selected = await open({ directory: true, title: t("collab.pickProjectDir") });
      if (selected) {
        const dir = selected as string;
        setNewProject(dir);
        if (!newName.trim()) setNewName(dir.split(/[\\/]/).pop() || "");
      }
    } catch {
      /* ignore */
    }
  };

  const createRoom = async () => {
    if (!newProject.trim()) {
      theamedAlert(t("collab.needProjectDir"));
      return;
    }
    const room = await invoke<CollabRoomT>("collab_create_room", {
      name: newName || t("collab.unnamedSession"),
      projectPath: newProject.trim(),
    });
    await refreshRooms();
    setActiveRoom(room);
    setCreating(false);
    setNewName("");
    setNewProject("");
  };

  const deleteRoom = async (id: string) => {
    try {
      await invoke("collab_delete_room", { roomId: id });
    } catch (e) {
      console.error("[CollabRoomT] 删除房间失败:", e);
    }
    await refreshRooms();
    if (activeRoom?.id === id) setActiveRoom(null);
  };
  return {
    handlePickProject,
    createRoom,
    deleteRoom,
  };
}
