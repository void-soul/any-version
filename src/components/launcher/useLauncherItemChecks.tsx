import { listen } from "@tauri-apps/api/event";
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Item, ItemCheckResult } from "./types";
import { theamedConfirm } from "../shared/ThemedAlert";
import type { Dispatch, SetStateAction } from "react";

/** Owns the LauncherItemChecks feature, independently of page rendering. */
export function useLauncherItemChecks({ allItems, showToast, setAllItems }: {
  allItems: Item[];
  showToast: (msg: string) => void;
  setAllItems: Dispatch<SetStateAction<Item[]>>;
}) {
  const { t } = useTranslation();


  // 检测状态：itemId -> 检测结果（红框标识）
  const [checking, setChecking] = useState(false);

  const [checkResults, setCheckResults] = useState<Record<number, { exists: boolean }>>({});

  const [missingCount, setMissingCount] = useState(0);

  // 检测进度：null 表示不在检测中
  const [checkProgress, setCheckProgress] = useState<{ done: number; total: number; name: string } | null>(null);


  // 从持久化的检测结果恢复红框标识（data.exists 为 false 的项目）
  useEffect(() => {
    if (allItems.length === 0) return;
    const restored: Record<number, { exists: boolean }> = {};
    let missing = 0;
    for (const it of allItems) {
      if (it.data.exists === false) {
        restored[it.id] = { exists: false };
        missing++;
      }
    }
    setCheckResults(restored);
    setMissingCount(missing);
  }, [allItems]);


  // 停止检测
  const stopCheck = () => {
    invoke("launcher_stop_check").catch((e) => console.error("停止检测失败:", e));
  };


  // 一键清理无效项目（检测为不存在），删除前询问确认
  const cleanupInvalidItems = async () => {
    if (missingCount === 0 && !allItems.some((it) => it.data.exists === false)) {
      showToast(t("launcher.noInvalid"));
      return;
    }
    const invalidCount = allItems.filter((it) => it.data.exists === false).length;
    const ok = await theamedConfirm(t("launcher.confirmDeleteInvalid", { count: invalidCount }), {
      danger: true,
    });
    if (!ok) return;
    try {
      const deleted = await invoke<number>("launcher_delete_invalid_items");
      // 同步前端状态：移除已删除项目，清空红框标识
      setAllItems((prev) => prev.filter((it) => it.data.exists !== false));
      setCheckResults({});
      setMissingCount(0);
      showToast(t("launcher.deletedInvalid", { count: deleted }));
    } catch (e) {
      console.error("清理无效项目失败", e);
      showToast(t("launcher.cleanFail"));
    }
  };


  // 检测所有项目是否存在（网页类刷新图标，其余红框标识缺失）
  const runCheck = async () => {
    if (allItems.length === 0) return;
    setChecking(true);
    setCheckProgress({ done: 0, total: allItems.length, name: "" });
    setMissingCount(0);
    setCheckResults({});
    try {
      const results = await invoke<ItemCheckResult[]>("launcher_check_items", {
        items: allItems,
      });
      // 兜底：一次性同步所有结果（兼容未收到部分事件的情形）
      const newResults: Record<number, { exists: boolean }> = {};
      let missing = 0;
      for (const r of results) {
        newResults[r.itemId] = { exists: r.exists };
        if (!r.exists) missing++;
      }
      setCheckResults(newResults);
      setMissingCount(missing);
      showToast(t("launcher.checkDone", { detail: missing > 0 ? t("launcher.checkDoneMissing", { count: missing }) : t("launcher.checkDoneOk") }));
    } catch (e) {
      console.error("检测失败", e);
      showToast(t("launcher.checkFail"));
    } finally {
      setChecking(false);
      setCheckProgress(null);
    }
  };


  // 监听检测进度事件（后端每完成一项 emit 一次，实时逐项呈现结果）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen<{
        done: number;
        total: number;
        itemId: number;
        name: string;
        exists: boolean;
        icon?: string | null;
        title?: string | null;
        stopped: boolean;
      }>("launcher-check-progress", (event) => {
        const p = event.payload;
        setCheckProgress({ done: p.done, total: p.total, name: p.name });

        // 用户停止：结束检测
        if (p.stopped) {
          setChecking(false);
          setCheckProgress(null);
          showToast(t("launcher.checkStopped"));
          return;
        }

        // 立即呈现该项结果：缺失→红框；网页类→更新图标/标题
        setCheckResults((prev) => ({ ...prev, [p.itemId]: { exists: p.exists } }));
        setMissingCount((prev) => (p.exists ? prev : prev + 1));

        if (p.icon || p.title) {
          setAllItems((prev) =>
            prev.map((it) =>
              it.id === p.itemId
                ? {
                    ...it,
                    name: p.title ? p.title : it.name,
                    data: {
                      ...it.data,
                      icon: p.icon ? p.icon : it.data.icon,
                    },
                  }
                : it
            )
          );
        }
      });
    };
    setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  return {
    checking,
    setChecking,
    checkResults,
    setCheckResults,
    missingCount,
    setMissingCount,
    checkProgress,
    setCheckProgress,
    stopCheck,
    cleanupInvalidItems,
    runCheck,
  };
}
