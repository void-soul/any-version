import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { progressKeyOf, progressRowsOf } from "./progress";
import { toast } from "../shared/Toast";
import {
  type CheckResult,
  type ClassifyResult,
  type FavoritesProgress,
  type ImportResult,
  type ImportState,
  type XAccount,
} from "./types";
import { isImportTask, type FavTask } from "./FavoritesPanel.model";
import { pickXImportChannel } from "./xImportChannel";

/** Owns the FavoriteTasks feature, independently of page rendering. */
export function useFavoriteTasks({ refresh, providerId, modelId }: {
  refresh: () => Promise<void>;
  providerId: string;
  modelId: string;
}) {
  const { t } = useTranslation();

  // 正在跑的长任务（替换原先单个 busy 字段）：三个导入可以同时在列，
  // 归类 / 失效检测与导入互斥，所以加工类任务在列时导入按钮全灰。
  const [running, setRunning] = useState<FavTask[]>([]);

  const startTask = (task: FavTask) =>
    setRunning((prev) => (prev.includes(task) ? prev : [...prev, task]));

  const endTask = (task: FavTask) => setRunning((prev) => prev.filter((x) => x !== task));

  /** 有导入在跑 → 归类/检测禁用；有加工在跑 → 导入禁用。 */
  const importRunning = running.some(isImportTask);

  const processRunning = running.includes("classify") || running.includes("check");

  // 导入 / 归类的实时进度（后端 favorites-progress 事件）。
  // 按任务分行存：三个导入同时跑时各显示各的，不会互相覆盖。
  const [progressMap, setProgressMap] = useState<Record<string, FavoritesProgress>>({});

  const clearProgress = (task: FavTask) =>
    setProgressMap((prev) => {
      if (!(task in prev)) return prev;
      const next = { ...prev };
      delete next[task];
      return next;
    });

  useEffect(() => {
    const unlisten = listen<FavoritesProgress>("favorites-progress", (event) => {
      const p = event.payload;
      const key = progressKeyOf(p);
      setProgressMap((prev) =>
        p.done
          ? Object.fromEntries(Object.entries(prev).filter(([k]) => k !== key))
          : { ...prev, [key]: p },
      );
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  /** 正在跑的任务的进度行（保持事件到达顺序）。 */
  const progressRows = progressRowsOf(progressMap, running);

  // B站：需要 Cookie（含 SESSDATA）才能读自己的收藏；配过就不再每次问
  const [biliConfigured, setBiliConfigured] = useState(false);

  const [cookieOpen, setCookieOpen] = useState(false);

  // 知乎：只走 Cookie 直连（官方接口拿不到私有收藏夹，已下线）
  const [zhihuCookieConfigured, setZhihuCookieConfigured] = useState(false);

  const [zhihuCookieOpen, setZhihuCookieOpen] = useState(false);

  // X：Cookie + 接口 ID 两件套（接口 ID 会随 X 前端发版变化，需可覆盖）
  const [xCookieConfigured, setXCookieConfigured] = useState(false);

  const [xCookieOpen, setXCookieOpen] = useState(false);

  const [xQueryIdConfigured, setXQueryIdConfigured] = useState(false);

  const [xQueryIdOpen, setXQueryIdOpen] = useState(false);

  // X 官方 API：授权一次后优先走官方（稳定、只读），未授权再回落到 Cookie
  const [xOauth, setXOauth] = useState<XAccount | null>(null);

  const [xOauthOpen, setXOauthOpen] = useState(false);

  /** 上次读取时间：官方接口不返回收藏时间，只能记录本地同步时间（增量判断也靠它） */
  const [xLastRunAt, setXLastRunAt] = useState<string | null>(null);

  /** 刷新 X 的授权状态与上次读取时间（面板打开、授权完成、导入结束后调用） */
  const refreshXStatus = useCallback(async () => {
    const [account, state] = await Promise.all([
      invoke<XAccount | null>("fav_x_oauth_status").catch(() => null),
      invoke<ImportState | null>("fav_import_state", { source: "x" }).catch(() => null),
    ]);
    setXOauth(account);
    setXLastRunAt(state?.lastRunAt ?? null);
  }, []);

  // GitHub Token：**收藏模块自己的一份**，与 SDK 模块的 token 互不共享
  const [tokenConfigured, setTokenConfigured] = useState(false);

  const [tokenOpen, setTokenOpen] = useState(false);

  const runImport = async () => {
    // 没配 Token 就直接把配置弹窗递上去，别让用户吃一个报错再自己找入口
    if (!tokenConfigured) {
      setTokenOpen(true);
      return;
    }
    startTask("github");
    try {
      const result = await invoke<ImportResult>("fav_import_github", { maxPages: null });
      await refresh();
      if (result.cancelled) {
        toast(t("favorites.importCancelled"), "ok");
      } else {
        toast(
          t("favorites.importDone", {
            added: result.added,
            updated: result.updated,
            skipped: result.skipped,
          }),
          "ok",
        );
      }
      reportSkippedDeleted(result.skippedDeleted);
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
    } finally {
      endTask("github");
      clearProgress("github");
    }
  };

  /** 导入完成后补一句「有几条是你之前删过的，已跳过」——否则用户会以为导入漏了 */
  const reportSkippedDeleted = (n?: number) => {
    if (!n) return;
    toast(t("favorites.importSkippedDeleted", { count: n }), "info");
  };

  const runImportBili = async () => {
    startTask("bilibili");
    try {
      const result = await invoke<ImportResult>("fav_import_bilibili");
      await refresh();
      toast(
        t("favorites.importDone", {
          added: result.added,
          updated: result.updated,
          skipped: result.skipped,
        }),
        "ok",
      );
      reportSkippedDeleted(result.skippedDeleted);
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
      // Cookie 失效是最常见原因：直接把配置弹窗递上去
      if (!biliConfigured) setCookieOpen(true);
    } finally {
      endTask("bilibili");
      clearProgress("bilibili");
    }
  };

  // 知乎：只走 Cookie 直连（含私有收藏夹；官方接口拿不到全量，已下线）
  const runImportZhihu = async () => {
    if (!zhihuCookieConfigured) {
      setZhihuCookieOpen(true);
      return;
    }
    startTask("zhihu");
    try {
      const result = await invoke<ImportResult>("fav_import_zhihu");
      await refresh();
      // 部分收藏夹失败（限流等）：主体照常导入，但要把失败明细露出来
      if (result.failed && result.failed.length > 0) {
        toast(
          t("favorites.importPartial", {
            added: result.added,
            failed: result.failed.length,
            detail: result.failed.join("；"),
          }),
          "err",
        );
      } else if (result.cancelled) {
        toast(t("favorites.importCancelled"), "ok");
      } else {
        toast(
          t("favorites.importDone", {
            added: result.added,
            updated: result.updated,
            skipped: result.skipped,
          }),
          "ok",
        );
      }
      reportSkippedDeleted(result.skippedDeleted);
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
      // Cookie 失效是最常见原因：把配置弹窗递上去
      setZhihuCookieOpen(true);
    } finally {
      endTask("zhihu");
      clearProgress("zhihu");
    }
  };

  // X：官方授权优先；没有官方凭证才用 Cookie + 接口 ID。两边都没配就引导去授权。
  const runImportX = async () => {
    const channel = pickXImportChannel({
      cookieReady: xCookieConfigured && xQueryIdConfigured,
      officialReady: xOauth != null,
    });
    if (channel === "none") {
      setXOauthOpen(true);
      return;
    }
    startTask("x");
    try {
      const result = await invoke<ImportResult>(
        channel === "official" ? "fav_import_x_official" : "fav_import_x",
      );
      await refresh();
      await refreshXStatus();
      if (result.cancelled) {
        toast(t("favorites.importCancelled"), "ok");
      } else {
        toast(
          t("favorites.importDone", {
            added: result.added,
            updated: result.updated,
            skipped: result.skipped,
          }),
          "ok",
        );
      }
      reportSkippedDeleted(result.skippedDeleted);
    } catch (e) {
      const message = String(e);
      toast(t("favorites.importFail", { err: message }), "err");
      // 授权失效 / 接口 ID 换版是最常见的两类失败：直接递对应的配置弹窗
      if (message.includes("授权") || message.includes("Client ID")) setXOauthOpen(true);
      else if (message.includes("接口 ID")) setXQueryIdOpen(true);
      else setXCookieOpen(true);
    } finally {
      endTask("x");
      clearProgress("x");
    }
  };

  /** 浏览器收藏夹导入：书签目录会按层级建成多级分类（可反复导入，靠 URL 去重） */
  const runImportBookmarks = async (browser: "edge" | "chrome") => {
    startTask("bookmark");
    try {
      const result = await invoke<{
        imported: number;
        folders: number;
        skipped: number;
        /** 命中删除墓碑而跳过（之前删过这个网址） */
        deleted?: number;
        file: string;
      }>("fav_import_bookmarks", { browser, customPath: null });
      await refresh();
      toast(
        t("favorites.importBookmarksDone", {
          browser: browser === "edge" ? "Edge" : "Chrome",
          count: result.imported,
          folders: result.folders,
        }),
        "ok",
      );
      reportSkippedDeleted(result.deleted);
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
    } finally {
      endTask("bookmark");
      clearProgress("bookmark");
    }
  };

  const runClassify = async (reclassify: boolean) => {
    // 双保险：按钮已禁用，但键盘/脚本路径也要挡住——后端同样会拒绝
    if (!providerId || !modelId) {
      toast(t("favorites.needProviderModel"), "info");
      return;
    }
    startTask("classify");
    try {
      const result = await invoke<ClassifyResult>("fav_classify", {
        providerId,
        modelId,
        limit: null,
        reclassify,
      });
      await refresh();
      if (result.cancelled) {
        // 停掉了也要说清「已归类多少条被保留」，否则用户不知道白干了没有
        toast(
          t(
            reclassify
              ? "favorites.classifyReclassifyCancelled"
              : "favorites.classifyCancelled",
            {
              classified: result.classified,
              remaining: result.remaining,
            },
          ),
          "info",
        );
      } else {
        toast(
          t(reclassify ? "favorites.classifyReclassifyDone" : "favorites.classifyDone", {
            classified: result.classified,
            tags: result.tagsWritten,
            remaining: result.remaining,
          }),
          "ok",
        );
      }
    } catch (e) {
      toast(t("favorites.classifyFail", { err: String(e) }), "err");
    } finally {
      endTask("classify");
      clearProgress("classify");
    }
  };

  const runCheck = async () => {
    startTask("check");
    try {
      const result = await invoke<CheckResult>("fav_check_gone", { all: false });
      await refresh();
      // 没配 Token 时 GitHub 那批查不了：如实说，不要让人以为「全查完了」
      const skipped = result.skippedNoToken
        ? ` · ${t("favorites.checkSkippedNoToken", { count: result.skippedNoToken })}`
        : "";
      if (result.cancelled) {
        toast(t("favorites.checkCancelled", { checked: result.checked }), "info");
      } else if (result.aborted) {
        toast(t("favorites.checkAborted", { checked: result.checked }), "err");
      } else {
        toast(
          t("favorites.checkDone", {
            checked: result.checked,
            gone: result.gone,
            redirect: result.redirect,
            unknown: result.unknown,
          }) + skipped,
          "ok",
        );
      }
    } catch (e) {
      toast(t("favorites.checkFail", { err: String(e) }), "err");
    } finally {
      endTask("check");
      clearProgress("check");
    }
  };

  /**
   * 停止当前长任务。
   *
   * 后端在**循环的下一轮开始前**检查标志：进行中的那次请求会跑完
   * （导入的一页 / 归类的一批 / 检测的一条），所以提示语要说清是「下一轮才停」，
   * 否则用户看界面没立刻反应会以为按钮坏了、再点几次。
   */
  const requestStop = async (task?: FavTask) => {
    try {
      await invoke("fav_cancel", { task: task ?? null });
      toast(t("favorites.stopRequested"), "info");
    } catch (e) {
      toast(String(e), "err");
    }
  };
  return {
    running,
    importRunning,
    processRunning,
    progressRows,
    biliConfigured,
    setBiliConfigured,
    cookieOpen,
    setCookieOpen,
    zhihuCookieConfigured,
    setZhihuCookieConfigured,
    zhihuCookieOpen,
    setZhihuCookieOpen,
    xCookieConfigured,
    setXCookieConfigured,
    xCookieOpen,
    setXCookieOpen,
    xQueryIdConfigured,
    setXQueryIdConfigured,
    xQueryIdOpen,
    setXQueryIdOpen,
    xOauth,
    setXOauth,
    xOauthOpen,
    setXOauthOpen,
    xLastRunAt,
    refreshXStatus,
    tokenConfigured,
    setTokenConfigured,
    tokenOpen,
    setTokenOpen,
    runImport,
    runImportBili,
    runImportZhihu,
    runImportX,
    runImportBookmarks,
    runClassify,
    runCheck,
    requestStop,
  };
}

/** 供子组件复用的返回类型（导入菜单抽到独立文件后按整个对象传递） */
export type FavoriteTasks = ReturnType<typeof useFavoriteTasks>;
