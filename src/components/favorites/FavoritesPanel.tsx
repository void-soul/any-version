// 收藏 / 星标聚合：只读导入各平台的收藏 → 本地库 → AI 多标签归类 → 失效检测。
//
// 设计要点：
// - 导入是幂等的（后端按平台原生 id 去重），重复点「导入」只会得到 added=0；
// - 归类只处理未归类且未被人工改过的条目，人工改标签后该条目被锁定；
// - 一个条目可以属于多个分类（多标签），所以同一条目会在多个分类下出现。
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  Clock,
  ExternalLink,
  Pencil,
  RefreshCw,
  Search,
  Plus,
  X,
  Trash2,
  Bot,
  Send,
  Sparkles,
} from "lucide-react";

import { favoritedDateLabel, sinceToLocalString, type FavoritesSort, type SincePreset } from "./favoritedTime";
import { parseAiResultLine } from "./aiResult";
import { SharedButton } from "../shared/Button";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { toast } from "../shared/Toast";
import { GithubTokenDialog } from "../project/GithubTokenDialog";
import { MarkdownRenderer } from "../ai/MarkdownRenderer";
import { CredentialDialog } from "./CredentialDialog";
import type { AiConfig, AiProvider } from "../ai/types";
import {
  SOURCE_LABELS,
  expiringInDays,
  statusBadge,
  type CachedContent,
  type CheckResult,
  type ClassifyResult,
  type CredentialStatus,
  type FavoriteRow,
  type FavoriteSettings,
  type FavoriteStats,
  type FavoriteCategoryNode,
  type FavoritesProgress,
  type ImportResult,
} from "./types";

/** 分类栏宽度：默认 180px（比原先固定的 160px 宽 20px），范围与后端 clamp 一致。 */
const DEFAULT_LEFT_WIDTH = 180;

/**
 * 长任务名（与后端 `commands/favorites/commands.rs` 的 TASK_* 一致）。
 *
 * 三个平台的导入可以同时跑（各拉各的接口），但导入与「加工」（归类 / 失效检测）
 * 必须互斥：加工要扫全库挑条目，和正在写入的导入抢同一份数据。
 */
type FavTask = "github" | "bilibili" | "zhihu" | "bookmark" | "classify" | "check";
const IMPORT_TASKS: FavTask[] = ["github", "bilibili", "zhihu", "bookmark"];
const isImportTask = (task: FavTask) => IMPORT_TASKS.includes(task);
const MIN_LEFT_WIDTH = 140;
const MAX_LEFT_WIDTH = 420;

export default function FavoritesPanel() {
  const { t } = useTranslation();

  const [items, setItems] = useState<FavoriteRow[]>([]);
  const [stats, setStats] = useState<FavoriteStats | null>(null);
  // 分类筛选：走分类 id（树），名字只用于回显
  const [categoryId, setCategoryId] = useState<number | null>(null);
  const [categoryName, setCategoryName] = useState<string | null>(null);
  // 分类树里展开的节点
  const [expandedCats, setExpandedCats] = useState<Set<number>>(new Set());
  // 分类右键菜单
  const [catMenu, setCatMenu] = useState<{ id: number | null; name: string; x: number; y: number } | null>(null);
  // 条目分类选择器
  const [pickerFor, setPickerFor] = useState<FavoriteRow | null>(null);
  const [pickerSelected, setPickerSelected] = useState<number[]>([]);
  const [source, setSource] = useState<string | null>(null);
  const [keyword, setKeyword] = useState("");
  // 排序默认按「收藏时间」：这是收藏模块，用户最关心的是「我什么时候收藏的」，
  // 而默认的「最近更新」会把 AI 归类动过的老条目顶到最前面，反直觉。
  const [sort, setSort] = useState<FavoritesSort>("favorited");
  // 收藏时间过滤（全部 / 近 7 天 / 近 30 天 / 近一年）
  const [since, setSince] = useState<SincePreset>("all");
  // 正在跑的长任务（替换原先单个 busy 字段）：三个导入可以同时在列，
  // 归类 / 失效检测与导入互斥，所以加工类任务在列时导入按钮全灰。
  const [running, setRunning] = useState<FavTask[]>([]);
  const startTask = (task: FavTask) =>
    setRunning((prev) => (prev.includes(task) ? prev : [...prev, task]));
  const endTask = (task: FavTask) => setRunning((prev) => prev.filter((x) => x !== task));
  /** 有导入在跑 → 归类/检测禁用；有加工在跑 → 导入禁用。 */
  const importRunning = running.some(isImportTask);
  const processRunning = running.includes("classify") || running.includes("check");

  // AI 归类的模型选择：直接复用 AI 模块的配置，不另设一套
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");

  // 左侧分类栏宽度：可拖动，宽度与模型选择一起存进 favorites_settings.json
  const [leftWidth, setLeftWidth] = useState(DEFAULT_LEFT_WIDTH);
  // 设置的内存副本：拖动/切模型都是「读-改-写」，先攒在这里再整份落盘
  const settingsRef = useRef<FavoriteSettings>({
    leftWidth: DEFAULT_LEFT_WIDTH,
    providerId: null,
    modelId: null,
  });

  /** 保存界面设置（整份覆盖，未传的字段沿用内存里的现值）。失败只记日志：设置丢了不影响功能。 */
  const persistSettings = useCallback(async (patch: Partial<FavoriteSettings>) => {
    settingsRef.current = { ...settingsRef.current, ...patch };
    try {
      await invoke("fav_save_settings", { settings: settingsRef.current });
    } catch (e) {
      console.error("保存收藏设置失败:", e);
    }
  }, []);

  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);

  // ── AI 检索助手（用户说需求 → agent 在本地库里找 → 整理成清单）──
  const [aiOpen, setAiOpen] = useState(false);
  const [aiInput, setAiInput] = useState("");
  const [aiResult, setAiResult] = useState("");
  const [aiSteps, setAiSteps] = useState<{ step: string; text: string; tool?: string }[]>([]);
  const [aiBusy, setAiBusy] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const aiLogRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const unlisten = listen<{ step: string; text: string; tool?: string }>(
      "favorites-agent-progress",
      (event) => {
        const s = event.payload;
        if (s.step === "thinking" || s.step === "done") return;
        setAiSteps((prev) => [...prev, s]);
      },
    );
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const el = aiLogRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [aiSteps]);

  const runAiSearch = async () => {
    const text = aiInput.trim();
    if (!text || aiBusy) return;
    setAiBusy(true);
    setAiError(null);
    setAiResult("");
    setAiSteps([]);
    try {
      const reply = await invoke<{ text: string }>("fav_agent_search", {
        providerId: providerId || null,
        modelId: modelId || null,
        prompt: text,
      });
      setAiResult(reply.text);
    } catch (e: any) {
      setAiError(String(e));
    } finally {
      setAiBusy(false);
    }
  };

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
      const key = p.task || p.stage;
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
  const progressRows = Object.entries(progressMap).filter(([key]) =>
    running.includes(key as FavTask),
  );

  // AI 归类的模型选择：配置里没存模型列表的供应商，现拉一次并按 provider 缓存
  const [fetchedModels, setFetchedModels] = useState<Record<string, string[]>>({});
  const [modelsLoading, setModelsLoading] = useState(false);

  // B站：需要 Cookie（含 SESSDATA）才能读自己的收藏；配过就不再每次问
  const [biliConfigured, setBiliConfigured] = useState(false);
  const [cookieOpen, setCookieOpen] = useState(false);

  // 知乎：只走 Cookie 直连（官方接口拿不到私有收藏夹，已下线）
  const [zhihuCookieConfigured, setZhihuCookieConfigured] = useState(false);
  const [zhihuCookieOpen, setZhihuCookieOpen] = useState(false);

  // 凭证健康状态：Cookie 快过期/已失效时主动提醒
  const [credStatus, setCredStatus] = useState<CredentialStatus[]>([]);
  const [credWarned, setCredWarned] = useState(false);

  // 展开看正文：知乎收藏内容 / GitHub README（懒抓，抓过就缓存在库里）
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [content, setContent] = useState<Record<number, CachedContent | null>>({});
  const [contentBusy, setContentBusy] = useState<number | null>(null);

  // GitHub Token：**收藏模块自己的一份**，与 SDK 模块的 token 互不共享
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const [tokenOpen, setTokenOpen] = useState(false);

  const refresh = useCallback(async () => {
    const [list, overview, creds] = await Promise.all([
      invoke<FavoriteRow[]>("fav_list", {
        source,
        // 分类改成树之后按 id 筛（后端会把子分类一起算进来）
        categoryId: categoryId ?? undefined,
        status: null,
        keyword: keyword.trim() || null,
        sort,
        // 时区换算放在前端：后端只拿到一个「不早于」的本地时间串，不需要猜时区
        favoritedSince: sinceToLocalString(since),
        limit: 0,
      }),
      invoke<FavoriteStats>("fav_stats"),
      invoke<CredentialStatus[]>("fav_credential_status"),
    ]);
    setItems(list);
    setStats(overview);
    setCredStatus(creds);
  }, [source, categoryId, keyword, sort, since]);

  // ── 分类树操作（对齐启动模块：新建子分类 / 重命名 / 删除 / 同级排序）──
  const cats: FavoriteCategoryNode[] = stats?.categories || [];

  const findCat = (id: number | null): FavoriteCategoryNode | null => {
    if (id == null) return null;
    const walk = (list: FavoriteCategoryNode[]): FavoriteCategoryNode | null => {
      for (const c of list) {
        if (c.id === id) return c;
        const hit = walk(c.children);
        if (hit) return hit;
      }
      return null;
    };
    return walk(cats);
  };

  /** 按名字找分类（条目标签存的是名字，筛选要的是 id） */
  const findCatByName = (name: string): FavoriteCategoryNode | null => {
    const walk = (list: FavoriteCategoryNode[]): FavoriteCategoryNode | null => {
      for (const c of list) {
        if (c.name === name) return c;
        const hit = walk(c.children);
        if (hit) return hit;
      }
      return null;
    };
    return walk(cats);
  };

  /** 同级排序用的 id 列表（上下移动时只重排这一层） */
  const siblingsOf = (id: number | null): FavoriteCategoryNode[] => {
    if (id == null) return cats;
    const walk = (list: FavoriteCategoryNode[]): FavoriteCategoryNode[] | null => {
      for (const c of list) {
        if (c.id === id) return list;
        const hit = walk(c.children);
        if (hit) return hit;
      }
      return null;
    };
    return walk(cats) || [];
  };

  const createCat = async (parentId: number | null) => {
    const name = window.prompt(t(pickCatKey(parentId)));
    if (!name?.trim()) return;
    try {
      const id = await invoke<number>("fav_create_category", { name: name.trim(), parentId });
      if (parentId != null) setExpandedCats((s) => new Set(s).add(parentId));
      setCategoryId(id);
      setCategoryName(name.trim());
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };
  function pickCatKey(parentId: number | null) {
    return parentId == null ? "favorites.newCategoryPrompt" : "favorites.newSubCategoryPrompt";
  }

  const renameCat = async (id: number) => {
    const cur = findCat(id);
    const name = window.prompt(t("favorites.renameCategoryPrompt"), cur?.name || "");
    if (!name?.trim()) return;
    try {
      await invoke("fav_rename_category", { id, name: name.trim() });
      if (categoryId === id) setCategoryName(name.trim());
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const deleteCat = async (id: number) => {
    const cur = findCat(id);
    if (!window.confirm(t("favorites.deleteCategoryConfirm", { name: cur?.name || "" }))) return;
    try {
      await invoke("fav_delete_category", { id });
      if (categoryId === id) {
        setCategoryId(null);
        setCategoryName(null);
      }
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** 同级上移 / 下移：只重排这一层，动完立刻落盘 */
  const moveCat = async (id: number, dir: -1 | 1) => {
    const list = siblingsOf(id);
    const from = list.findIndex((c) => c.id === id);
    const to = from + dir;
    if (from < 0 || to < 0 || to >= list.length) return;
    const next = [...list];
    [next[from], next[to]] = [next[to], next[from]];
    try {
      await invoke("fav_reorder_categories", {
        orders: next.map((c, i) => [c.id, i]),
      });
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  // ── 条目分类选择器 ──
  const openPicker = (item: FavoriteRow) => {
    setPickerFor(item);
    const ids = (item.tags || [])
      .map((name) => cats.find((c) => c.name === name)?.id)
      .filter((v): v is number => typeof v === "number");
    setPickerSelected(ids);
  };

  const submitPicker = async () => {
    if (!pickerFor) return;
    try {
      await invoke("fav_set_item_categories", { id: pickerFor.id, categoryIds: pickerSelected });
      setPickerFor(null);
      await refresh();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    void refresh().catch((e) => toast(String(e), "err"));
  }, [refresh]);

  useEffect(() => {
    invoke<boolean>("fav_has_credential", { source: "bilibili" })
      .then(setBiliConfigured)
      .catch(() => setBiliConfigured(false));
    invoke<boolean>("fav_has_credential", { source: "zhihu-cookie" })
      .then(setZhihuCookieConfigured)
      .catch(() => setZhihuCookieConfigured(false));
    invoke<string>("fav_get_github_token")
      .then((token) => setTokenConfigured(!!token.trim()))
      .catch(() => setTokenConfigured(false));
  }, []);

  /** 某个凭证的告警级别：expired > soon > ok/unknown（未配置则 null）。 */
  const credAlert = useCallback(
    (sourceKey: string): "expired" | "soon" | null => {
      const found = credStatus.find((c) => c.source === sourceKey);
      if (!found?.configured) return null;
      if (found.status === "expired") return "expired";
      if (expiringInDays(found.expiresAt) !== null) return "soon";
      return null;
    },
    [credStatus],
  );

  // 失效/快过期只主动提醒一次，别每次重渲染都弹
  useEffect(() => {
    if (credWarned || credStatus.length === 0) return;
    const expired = credStatus.filter((c) => c.configured && c.status === "expired");
    const soon = credStatus.filter(
      (c) => c.configured && c.status !== "expired" && expiringInDays(c.expiresAt) !== null,
    );
    if (expired.length === 0 && soon.length === 0) return;
    setCredWarned(true);
    if (expired.length > 0) {
      toast(
        t("favorites.credExpiredHint", {
          sources: expired.map((c) => t(`favorites.credName.${c.source}`)).join("、"),
        }),
        "err",
      );
    } else {
      const days = expiringInDays(soon[0].expiresAt) ?? 0;
      toast(
        t("favorites.credSoonHint", {
          source: t(`favorites.credName.${soon[0].source}`),
          days,
        }),
        "info",
      );
    }
  }, [credStatus, credWarned, t]);

  // 供应商/模型预填：优先收藏模块「上次用的」，其次全局默认（翻译模块里选的），
  // 再回退首个可用供应商——选过一次就不会每次进来又被重置成默认。
  useEffect(() => {
    let alive = true;
    void Promise.all([
      invoke<AiConfig>("get_ai_config"),
      invoke<FavoriteSettings>("fav_get_settings").catch(() => null),
      invoke<{ providerId: string | null; modelId: string | null }>("get_translate_config").catch(
        () => ({ providerId: null, modelId: null }),
      ),
    ])
      .then(([cfg, saved, globalDefault]) => {
        if (!alive) return;
        const all = cfg.providers || [];
        const usable = all.filter((p) => p.openai_url && p.api_key);
        setProviders(usable);

        // 分类栏宽度：坏值回默认，越界收敛（后端也会再钳一次）
        const savedWidth = saved?.leftWidth;
        const width =
          typeof savedWidth === "number" && Number.isFinite(savedWidth) && savedWidth > 0
            ? Math.min(MAX_LEFT_WIDTH, Math.max(MIN_LEFT_WIDTH, savedWidth))
            : DEFAULT_LEFT_WIDTH;
        setLeftWidth(width);
        settingsRef.current = {
          leftWidth: width,
          providerId: saved?.providerId ?? null,
          modelId: saved?.modelId ?? null,
        };

        // 供应商优先级：收藏上次用的 > 全局默认 > 首个可用 > 第一个
        const wantPid = saved?.providerId || globalDefault.providerId;
        const provider =
          (wantPid && all.find((p) => p.id === wantPid && p.openai_url && p.api_key)) ||
          usable[0] ||
          all[0];
        if (!provider) return;
        setProviderId(provider.id);

        // 模型优先级：同属该供应商的「上次用的」> 全局默认 > 供应商激活模型 > 第一个
        const wantMid =
          saved?.providerId === provider.id
            ? saved.modelId
            : globalDefault.providerId === provider.id
              ? globalDefault.modelId
              : null;
        setModelId(
          (wantMid && provider.models.some((m) => m.id === wantMid) ? wantMid : null) ??
            provider.active_model_id ??
            provider.models[0]?.id ??
            "",
        );
      })
      .catch(() => setProviders([]));
    return () => {
      alive = false;
    };
  }, []);

  const activeProvider = useMemo(
    () => providers.find((p) => p.id === providerId) ?? null,
    [providers, providerId],
  );

  /** 当前可选的模型：优先用配置里存的，没有就现拉（有些供应商配置里 models 是空的） */
  const modelOptions = useMemo<string[]>(() => {
    if (!activeProvider) return [];
    if (activeProvider.models.length > 0) {
      return activeProvider.models.map((m) => m.id);
    }
    return fetchedModels[activeProvider.id] ?? [];
  }, [activeProvider, fetchedModels]);

  // 模型列表为空时自动补拉一次（失败静默：用户仍可用「默认」让后端自己选）
  useEffect(() => {
    if (!activeProvider || activeProvider.models.length > 0) return;
    const pid = activeProvider.id;
    if (fetchedModels[pid]) return;
    setModelsLoading(true);
    invoke<string[]>("fetch_provider_models", {
      baseUrl: activeProvider.openai_url,
      apiKey: activeProvider.api_key,
      headers: activeProvider.custom_headers ?? [],
    })
      .then((models) => setFetchedModels((prev) => ({ ...prev, [pid]: models })))
      .catch(() => setFetchedModels((prev) => ({ ...prev, [pid]: [] })))
      .finally(() => setModelsLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeProvider?.id]);

  /** 拖动分类栏右侧分隔条：拖动过程只改内存宽度，松手才落盘（免得拖一次写几十遍文件）。 */
  const startLeftResize = (e: ReactMouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = leftWidth;
    let next = startWidth;
    const onMove = (ev: MouseEvent) => {
      next = Math.min(MAX_LEFT_WIDTH, Math.max(MIN_LEFT_WIDTH, startWidth + (ev.clientX - startX)));
      setLeftWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      if (next !== startWidth) void persistSettings({ leftWidth: next });
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

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
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
    } finally {
      endTask("github");
      clearProgress("github");
    }
  };

  /**
   * 展开正文：知乎用已缓存的内容，GitHub 懒抓 README（优先中文版）。
   *
   * `refresh` 为 true 时强制重新抓（GitHub 用），否则命中缓存直接显示。
   */
  const openContent = async (item: FavoriteRow, force = false) => {
    setContentBusy(item.id);
    try {
      const data =
        item.source === "github"
          ? await invoke<CachedContent>("fav_github_readme", { id: item.id, refresh: force })
          : await invoke<CachedContent | null>("fav_get_content", { id: item.id });
      setContent((prev) => ({ ...prev, [item.id]: data }));
    } catch (e) {
      toast(t("favorites.contentFail", { err: String(e) }), "err");
      // 失败也记一笔，避免每次收起再展开都重试一遍
      setContent((prev) => ({ ...prev, [item.id]: null }));
    } finally {
      setContentBusy(null);
    }
  };

  /** 能否展开看正文：知乎有正文缓存，GitHub 能抓 README，B站暂不支持。 */
  const canPreview = (item: FavoriteRow) =>
    item.source === "zhihu" || item.source === "github";

  const toggleContent = (item: FavoriteRow) => {
    if (expandedId === item.id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(item.id);
    if (!(item.id in content)) void openContent(item);
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
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
      // Cookie 失效是最常见原因：把配置弹窗递上去
      setZhihuCookieOpen(true);
    } finally {
      endTask("zhihu");
      clearProgress("zhihu");
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
    } catch (e) {
      toast(t("favorites.importFail", { err: String(e) }), "err");
    } finally {
      endTask("bookmark");
      clearProgress("bookmark");
    }
  };

  const runClassify = async () => {
    startTask("classify");
    try {
      const result = await invoke<ClassifyResult>("fav_classify", {
        providerId: providerId || null,
        modelId: modelId || null,
        limit: null,
      });
      await refresh();
      if (result.cancelled) {
        // 停掉了也要说清「已归类多少条被保留」，否则用户不知道白干了没有
        toast(
          t("favorites.classifyCancelled", {
            classified: result.classified,
            remaining: result.remaining,
          }),
          "info",
        );
      } else {
        toast(
          t("favorites.classifyDone", {
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
          }),
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

  const removeItem = (item: FavoriteRow) => {
    setConfirmRequest({
      title: t("favorites.deleteTitle"),
      desc: t("favorites.deleteConfirm", { name: item.title }),
      danger: true,
      onConfirm: async () => {
        setConfirmRequest(null);
        try {
          await invoke("fav_delete", { id: item.id });
          await refresh();
        } catch (e) {
          toast(t("favorites.deleteFail", { err: String(e) }), "err");
        }
      },
    });
  };

  const goneCount = stats?.gone ?? 0;

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

  /** 只在对应任务运行时出现的「停止」链接（导入各自停各自的，互不牵连）。 */
  const stopLink = (task: FavTask) =>
    running.includes(task) ? (
      <LinkButton label={t("favorites.stop")} danger onClick={() => void requestStop(task)} />
    ) : null;

  /** 凭证按钮的悬停提示：有告警时把原因说清楚，别让用户猜角标是什么意思。 */
  const credTitle = (sourceKey: string, fallback: string) => {
    const level = credAlert(sourceKey);
    if (level === "expired") return t("favorites.credExpired");
    if (level === "soon") {
      const days =
        expiringInDays(credStatus.find((c) => c.source === sourceKey)?.expiresAt) ?? 0;
      return t("favorites.credSoon", { days });
    }
    return fallback;
  };

  return (
    <div className="h-full flex flex-col gap-2 p-3 text-slate-200">
      {/* 顶部操作区：站点名只出现一次，动作一律用文字链接表达（细分隔线分组）。
          三个站点的两个动作语义一致（导入 / 密钥），凭证键不同所以互不覆盖。 */}
      <div className="flex items-center gap-x-3 gap-y-1 flex-wrap">
        {/* GitHub star */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">GitHub</span>
          <LinkButton
            label={t("favorites.import")}
            busy={running.includes("github")}
            disabled={running.includes("github") || processRunning}
            title={processRunning ? t("favorites.importBlockedByProcess") : undefined}
            onClick={() => void runImport()}
          />
          {stopLink("github")}
          <KeyLink
            label={t("favorites.key")}
            configured={tokenConfigured}
            title={
              tokenConfigured
                ? t("favorites.githubTokenSetTip")
                : t("favorites.githubTokenNeedTip")
            }
            onClick={() => setTokenOpen(true)}
          />
        </div>

        <Divider />

        {/* 浏览器收藏夹（从启动模块搬来）：书签目录会建成多级分类 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">{t("favorites.browserLabel")}</span>
          {(["edge", "chrome"] as const).map((b) => (
            <LinkButton
              key={b}
              label={b === "edge" ? "Edge" : "Chrome"}
              busy={running.includes("bookmark")}
              disabled={running.includes("bookmark") || processRunning}
              title={processRunning ? t("favorites.importBlockedByProcess") : t("favorites.importBookmarksHint")}
              onClick={() => void runImportBookmarks(b)}
            />
          ))}
        </div>

        <Divider />

        {/* B站收藏 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">B站</span>
          <LinkButton
            label={t("favorites.import")}
            busy={running.includes("bilibili")}
            disabled={running.includes("bilibili") || processRunning}
            title={processRunning ? t("favorites.importBlockedByProcess") : undefined}
            onClick={() => (biliConfigured ? void runImportBili() : setCookieOpen(true))}
          />
          {stopLink("bilibili")}
          <KeyLink
            label={t("favorites.key")}
            configured={biliConfigured}
            alert={credAlert("bilibili")}
            title={credTitle(
              "bilibili",
              biliConfigured
                ? t("favorites.biliCookieSetTip")
                : t("favorites.biliCookieNeedTip"),
            )}
            onClick={() => setCookieOpen(true)}
          />
        </div>

        <Divider />

        {/* 知乎收藏 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">知乎</span>
          <LinkButton
            label={t("favorites.import")}
            busy={running.includes("zhihu")}
            disabled={running.includes("zhihu") || processRunning}
            title={
              processRunning
                ? t("favorites.importBlockedByProcess")
                : t("favorites.zhihuImportHint")
            }
            onClick={() => void runImportZhihu()}
          />
          {stopLink("zhihu")}
          {/* 密钥**永远打开弹窗**（回显已存的 Cookie）：之前配过就直接跑的写法
              让用户再也进不去弹窗，改不了一份过期 Cookie。 */}
          <KeyLink
            label={t("favorites.key")}
            configured={zhihuCookieConfigured}
            alert={credAlert("zhihu-cookie")}
            title={credTitle(
              "zhihu-cookie",
              zhihuCookieConfigured
                ? t("favorites.zhihuCookieSetTip")
                : t("favorites.zhihuCookieNeedTip"),
            )}
            onClick={() => setZhihuCookieOpen(true)}
          />
        </div>

        <Divider />

        {/* AI 归类 + 失效检测：都是对已有的本地库做加工 */}
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] text-slate-200 font-medium">
            {t("favorites.toolsLabel")}
          </span>
          <select
            value={providerId}
            onChange={(e) => {
              const pid = e.target.value;
              setProviderId(pid);
              const p = providers.find((x) => x.id === pid);
              const mid = p?.active_model_id || p?.models[0]?.id || "";
              setModelId(mid);
              void persistSettings({ providerId: pid || null, modelId: mid || null });
            }}
            className="glass-input px-2 h-6 text-[11px] cursor-pointer max-w-[130px]"
            title={t("favorites.providerHint")}
          >
            <option value="">{t("favorites.providerDefault")}</option>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <select
            value={modelId}
            onChange={(e) => {
              setModelId(e.target.value);
              void persistSettings({
                providerId: providerId || null,
                modelId: e.target.value || null,
              });
            }}
            className="glass-input px-2 h-6 text-[11px] cursor-pointer max-w-[170px]"
            title={t("favorites.modelHint")}
            disabled={!activeProvider}
          >
            <option value="">{t("favorites.modelDefault")}</option>
            {modelsLoading && <option disabled>{t("favorites.modelLoading")}</option>}
            {modelOptions.map((id) => (
              <option key={id} value={id}>
                {id}
              </option>
            ))}
          </select>
          <LinkButton
            label={t("favorites.classify")}
            busy={running.includes("classify")}
            disabled={running.length > 0 || providers.length === 0}
            title={importRunning ? t("favorites.processBlockedByImport") : undefined}
            onClick={() => void runClassify()}
          />
          <LinkButton
            label={t("favorites.check")}
            busy={running.includes("check")}
            disabled={running.length > 0}
            title={importRunning ? t("favorites.processBlockedByImport") : undefined}
            onClick={() => void runCheck()}
          />
          {/* AI 检索：用自然语言说需求，agent 在本地收藏库里找并整理成清单 */}
          <LinkButton
            label={t("favorites.aiSearch")}
            busy={aiBusy}
            disabled={running.length > 0 || providers.length === 0}
            title={importRunning ? t("favorites.processBlockedByImport") : undefined}
            onClick={() => {
              setAiError(null);
              setAiResult("");
              setAiSteps([]);
              setAiOpen(true);
            }}
          />
          {/* 加工类同一时刻只可能有一个在跑，停它即可（不传 task 时会停全部） */}
          {processRunning && (
            <LinkButton
              label={t("favorites.stop")}
              danger
              onClick={() => void requestStop(running.find((x) => !isImportTask(x)))}
            />
          )}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <div className="flex items-center gap-1 glass-input px-2 h-7">
            <Search className="w-3 h-3 text-slate-500" />
            <input
              value={keyword}
              onChange={(e) => setKeyword(e.target.value)}
              placeholder={t("favorites.searchPlaceholder")}
              className="bg-transparent outline-none text-[11px] w-36"
            />
          </div>
          <select
            value={source ?? ""}
            onChange={(e) => setSource(e.target.value || null)}
            className="glass-input px-2 h-7 text-[11px] cursor-pointer"
          >
            <option value="">{t("favorites.allSources")}</option>
            {(stats?.bySource || []).map(([key]) => (
              <option key={key} value={key}>
                {SOURCE_LABELS[key] ?? key}
              </option>
            ))}
          </select>
          {/* 排序方式：默认按收藏时间 */}
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value as FavoritesSort)}
            className="glass-input px-2 h-7 text-[11px] cursor-pointer"
            title={t("favorites.sortTip")}
          >
            <option value="favorited">{t("favorites.sortFavorited")}</option>
            <option value="created">{t("favorites.sortCreated")}</option>
            <option value="updated">{t("favorites.sortUpdated")}</option>
          </select>
          {/* 收藏时间过滤 */}
          <select
            value={since}
            onChange={(e) => setSince(e.target.value as SincePreset)}
            className="glass-input px-2 h-7 text-[11px] cursor-pointer"
            title={t("favorites.sinceTip")}
          >
            <option value="all">{t("favorites.sinceAll")}</option>
            <option value="7d">{t("favorites.since7d")}</option>
            <option value="30d">{t("favorites.since30d")}</option>
            <option value="365d">{t("favorites.since365d")}</option>
          </select>
        </div>
      </div>

      {/* 实时进度：**每个在跑的任务一行** —— 三个导入可以同时跑，各显示各的，
          用一条进度条会让后到的事件盖掉前一个。任务结束由 done 事件（或 finally）清掉。 */}
      {progressRows.length > 0 && (
        <div className="space-y-1">
          {progressRows.map(([key, p]) => (
            <div key={key} className="glass-panel px-3 py-2 space-y-1.5">
              <div className="flex items-center gap-2 text-[10px] text-slate-300 flex-wrap">
                <RefreshCw className="w-3 h-3 animate-spin text-[var(--module-accent)]" />
                {p.stage === "check" ? (
                  <>
                    <span>{t("favorites.checkProgress")}</span>
                    <span>
                      {t("favorites.progressChecked", {
                        checked: p.checked ?? 0,
                        total: p.checkTotal ?? 0,
                      })}
                    </span>
                    <span className="text-slate-500 truncate max-w-[260px]">
                      {p.message ?? ""}
                    </span>
                  </>
                ) : p.stage === "import" ? (
                  <>
                    <span>
                      {SOURCE_LABELS[p.source ?? ""] ?? p.source ?? ""}
                      {p.folder ? ` · ${p.folder}` : ""}
                      {p.message ? ` · ${p.message}` : ""}
                    </span>
                    <span>{t("favorites.progressFetched", { fetched: p.fetched ?? 0 })}</span>
                    <span className="text-emerald-400/80">
                      {t("favorites.progressAdded", { added: p.added ?? 0 })}
                    </span>
                    <span className="text-amber-400/80">
                      {t("favorites.progressUpdated", { updated: p.updated ?? 0 })}
                    </span>
                    <span className="text-slate-500">
                      {t("favorites.progressSkipped", { skipped: p.skipped ?? 0 })}
                    </span>
                  </>
                ) : (
                  <>
                    <span>{p.message}</span>
                    <span>
                      {t("favorites.progressClassified", {
                        classified: p.classified ?? 0,
                        remaining: p.remaining ?? 0,
                      })}
                    </span>
                    <span className="text-slate-400">
                      {t("favorites.progressTags", { tags: p.tagsWritten ?? 0 })}
                    </span>
                  </>
                )}
              </div>
              <div className="h-1 rounded bg-white/5 overflow-hidden">
                {p.stage === "check" && (p.checkTotal ?? 0) > 0 ? (
                  <div
                    className="h-full bg-[var(--module-accent)] transition-all duration-300"
                    style={{
                      width: `${Math.min(
                        100,
                        ((p.checked ?? 0) / (p.checkTotal ?? 1)) * 100,
                      )}%`,
                    }}
                  />
                ) : p.stage === "classify" &&
                  (p.classified ?? 0) + (p.remaining ?? 0) > 0 ? (
                  <div
                    className="h-full bg-[var(--module-accent)] transition-all duration-300"
                    style={{
                      width: `${Math.min(
                        100,
                        ((p.classified ?? 0) /
                          ((p.classified ?? 0) + (p.remaining ?? 0))) *
                          100,
                      )}%`,
                    }}
                  />
                ) : p.stage === "import" &&
                  (p.folderTotal ?? 0) > 0 &&
                  (p.folderFetched ?? 0) > 0 ? (
                  // 知乎：服务端给了收藏夹总数（Totals），可以显示真实百分比
                  <div
                    className="h-full bg-[var(--module-accent)] transition-all duration-300"
                    style={{
                      width: `${Math.min(
                        100,
                        ((p.folderFetched ?? 0) / (p.folderTotal ?? 1)) * 100,
                      )}%`,
                    }}
                  />
                ) : (
                  <div className="h-full w-1/3 bg-[var(--module-accent)] animate-pulse" />
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="flex-1 flex min-h-0">
        {/* 左侧分类树（宽度可拖动，默认 180px，拖动后写回设置） */}
        <div
          className="shrink-0 overflow-y-auto glass-panel p-2 space-y-0.5"
          style={{ width: leftWidth }}
        >
          <button
            onClick={() => { setCategoryId(null); setCategoryName(null); }}
            className={`w-full text-left px-2 py-1 rounded text-[11px] cursor-pointer transition-colors ${
              categoryId === null
                ? "bg-[var(--module-accent-soft)] text-white"
                : "text-slate-400 hover:bg-white/5"
            }`}
          >
            {t("favorites.allTags")}
            <span className="float-right text-slate-500">{stats?.total ?? 0}</span>
          </button>

          {/* 分类树：可折叠，右键出菜单（新建子分类 / 重命名 / 上移下移 / 删除） */}
          <CategoryTree
            nodes={cats}
            depth={0}
            expanded={expandedCats}
            selectedId={categoryId}
            onToggle={(id) => setExpandedCats((s) => {
              const next = new Set(s);
              if (next.has(id)) next.delete(id); else next.add(id);
              return next;
            })}
            onSelect={(id, name) => { setCategoryId(id); setCategoryName(name); }}
            onContextMenu={(id, name, x, y) => setCatMenu({ id, name, x, y })}
          />

          <button
            onClick={() => void createCat(null)}
            className="w-full text-left px-2 py-1 mt-1 rounded text-[10px] text-slate-500 hover:text-slate-200 hover:bg-white/5 cursor-pointer"
            title={t("favorites.newCategory")}
          >
            <Plus className="w-3 h-3 inline mr-1" />
            {t("favorites.newCategory")}
          </button>

          {cats.length === 0 && (
            <p className="text-[10px] text-slate-500 px-2 py-1 leading-snug">
              {t("favorites.noTagsHint")}
            </p>
          )}
        </div>

        {/* 分隔条：左右布局的唯一调节点（拖动改左栏宽度） */}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t("favorites.resizeLeftTip")}
          title={t("favorites.resizeLeftTip")}
          onMouseDown={startLeftResize}
          className="w-2 shrink-0 cursor-col-resize rounded transition-colors hover:bg-[var(--module-accent-soft)]"
        />

        {/* 右侧条目列表 */}
        <div className="flex-1 overflow-y-auto glass-panel divide-y divide-white/5">
          {items.length === 0 && (
            <div className="h-full flex items-center justify-center text-[11px] text-slate-500">
              {t("favorites.empty")}
            </div>
          )}
          {items.map((item) => {
            const badge = statusBadge(item.status);
            return (
              <div key={item.id} className="px-3 py-2 hover:bg-white/5 group">
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <button
                        onClick={() => void openUrl(item.url).catch(() => toast(item.url, "err"))}
                        className="text-[12px] font-medium text-slate-100 hover:text-[var(--module-accent)] cursor-pointer truncate flex items-center gap-1"
                        title={item.url}
                      >
                        {item.title}
                        <ExternalLink className="w-2.5 h-2.5 opacity-0 group-hover:opacity-60" />
                      </button>
                      <span className="text-[9px] px-1 rounded bg-white/5 text-slate-500">
                        {SOURCE_LABELS[item.source] ?? item.source}
                      </span>
                      {/* 收藏时间：优先平台记录的时间；老库/平台不返回时回退到入库时间，
                          并在 tooltip 里说清这是哪一个，别让用户以为平台时间不准 */}
                      {(() => {
                        const platform = favoritedDateLabel(item.favoritedAt);
                        const label = platform ?? favoritedDateLabel(item.createdAt);
                        if (!label) return null;
                        return (
                          <span
                            className="text-[9px] px-1 rounded bg-white/5 text-slate-500 flex items-center gap-0.5 shrink-0"
                            title={
                              platform
                                ? t("favorites.favoritedAtTip", { time: item.favoritedAt })
                                : t("favorites.importedAtTip", { time: item.createdAt })
                            }
                          >
                            <Clock className="w-2.5 h-2.5" />
                            {label}
                          </span>
                        );
                      })()}
                      {badge && (
                        <span className={`text-[9px] px-1 rounded ${badge.className}`}>
                          {t(badge.text)}
                        </span>
                      )}
                      {item.aiLocked && (
                        <span
                          className="text-[9px] px-1 rounded bg-emerald-500/10 text-emerald-400/80"
                          title={t("favorites.lockedHint")}
                        >
                          {t("favorites.locked")}
                        </span>
                      )}
                    </div>
                    {item.description && (
                      <p className="text-[10px] text-slate-400 line-clamp-2 mt-0.5">
                        {item.description}
                      </p>
                    )}

                    <div className="flex items-center gap-1 mt-1 flex-wrap">
                      {item.tags.map((name) => {
                        const hit = findCatByName(name);
                        return (
                          <button
                            key={name}
                            onClick={() => {
                              setCategoryId(hit?.id ?? null);
                              setCategoryName(hit ? hit.name : name);
                            }}
                            className="text-[9px] px-1.5 py-0.5 rounded-full bg-white/5 text-slate-300 hover:bg-white/10 cursor-pointer"
                            title={hit ? t("favorites.filterByCategory") : name}
                          >
                            {name}
                          </button>
                        );
                      })}
                      <button
                        onClick={() => openPicker(item)}
                        className="p-0.5 rounded text-slate-600 hover:text-slate-300 cursor-pointer opacity-0 group-hover:opacity-100"
                        title={t("favorites.editTags")}
                      >
                        <Pencil className="w-2.5 h-2.5" />
                      </button>
                    </div>
                  </div>

                  <div className="flex items-center gap-0.5">
                    {canPreview(item) && (
                      <button
                        onClick={() => toggleContent(item)}
                        className="p-1 rounded text-slate-600 hover:text-slate-200 cursor-pointer"
                        title={
                          expandedId === item.id
                            ? t("favorites.contentHide")
                            : t("favorites.contentShow")
                        }
                      >
                        {expandedId === item.id ? (
                          <ChevronDown className="w-3 h-3" />
                        ) : (
                          <ChevronRight className="w-3 h-3" />
                        )}
                      </button>
                    )}
                    <button
                      onClick={() => removeItem(item)}
                      className="p-1 rounded text-slate-600 hover:text-rose-400 cursor-pointer opacity-0 group-hover:opacity-100"
                      title={t("favorites.delete")}
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>
                </div>

                {/* 展开的正文：知乎是导入时顺手缓存的内容，GitHub 是懒抓的 README（优先中文版） */}
                {expandedId === item.id && (
                  <div className="mt-1.5 rounded-lg border border-white/10 bg-black/25 p-2">
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-[9px] text-slate-500 truncate">
                        {contentBusy === item.id
                          ? t("favorites.contentLoading")
                          : content[item.id]?.label ??
                            (item.source === "github"
                              ? t("favorites.contentReadme")
                              : t("favorites.contentEmpty"))}
                        {content[item.id]?.fetchedAt
                          ? ` · ${content[item.id]?.fetchedAt}`
                          : ""}
                      </span>
                      {item.source === "github" && (
                        <button
                          onClick={() => void openContent(item, true)}
                          disabled={contentBusy === item.id}
                          className="ml-auto text-[9px] text-slate-500 hover:text-slate-200 cursor-pointer disabled:opacity-40"
                          title={t("favorites.contentRefresh")}
                        >
                          {t("favorites.contentRefresh")}
                        </button>
                      )}
                    </div>
                    {contentBusy === item.id ? (
                      <div className="flex items-center gap-2 text-[10px] text-slate-500 py-2">
                        <RefreshCw className="w-3 h-3 animate-spin" />
                        {t("favorites.contentLoading")}
                      </div>
                    ) : content[item.id]?.text ? (
                      <div className="max-h-72 overflow-y-auto">
                        {item.source === "github" ? (
                          <MarkdownRenderer content={content[item.id]!.text} />
                        ) : (
                          // 知乎正文只渲染纯文本：远端 HTML 直接进 DOM 等于把注入面交给知乎
                          <p className="text-[11px] text-slate-300 leading-relaxed whitespace-pre-wrap">
                            {content[item.id]!.text}
                          </p>
                        )}
                      </div>
                    ) : (
                      <p className="text-[10px] text-slate-500 py-1">
                        {t("favorites.contentEmpty")}
                      </p>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex items-center gap-3 text-[10px] text-slate-500">
        <span>
          {t("favorites.total")}: {stats?.total ?? 0}
        </span>
        <span>
          {t("favorites.unclassified")}: {stats?.unclassified ?? 0}
        </span>
        {goneCount > 0 && (
          <span className="text-rose-400/80">
            {t("favorites.goneCount")}: {goneCount}
          </span>
        )}
        {/* 当前分类筛选（含子分类）：点了分类树就得让用户知道自己「在哪一层」 */}
        {categoryId != null && categoryName && (
          <button
            onClick={() => { setCategoryId(null); setCategoryName(null); }}
            className="px-1.5 py-0.5 rounded-full bg-[var(--module-accent-soft)] text-[var(--module-accent)] cursor-pointer"
            title={t("favorites.clearCategoryFilter")}
          >
            {categoryName} ✕
          </button>
        )}
        <span className="ml-auto">{t("favorites.readonlyHint")}</span>
      </div>

      {/* GitHub Token（收藏模块专属）：与 SDK 模块的 Token 各存各的 */}
      <GithubTokenDialog
        open={tokenOpen}
        onClose={() => setTokenOpen(false)}
        onSaved={setTokenConfigured}
        getCommand="fav_get_github_token"
        setCommand="fav_set_github_token"
        titleKey="favorites.githubTokenTitle"
        hintKey="favorites.githubTokenHint"
        noteKey="favorites.githubTokenLocalNote"
      />

      {/* 两类凭证各用一个弹窗，**存储键不同所以互不覆盖**：
          - zhihu-cookie = 知乎 Cookie（多文本框，需含 z_c0 + d_c0）
          - bilibili     = B站 Cookie（多文本框，需含 SESSDATA） */}
      <CredentialDialog
        open={zhihuCookieOpen}
        onClose={() => setZhihuCookieOpen(false)}
        source="zhihu-cookie"
        title={t("favorites.zhihuCookieTitle")}
        hint={t("favorites.zhihuCookieHint")}
        placeholder={t("favorites.zhihuCookiePlaceholder")}
        note={t("favorites.zhihuCookieNote")}
        multiline
        onSaved={(configured) => {
          setZhihuCookieConfigured(configured);
          // 换了 Cookie 就把健康状态刷新一下：后端会把 status 重置为「未验证」
          void refresh().catch(() => {});
        }}
      />

      <CredentialDialog
        open={cookieOpen}
        onClose={() => setCookieOpen(false)}
        source="bilibili"
        title={t("favorites.biliCookieTitle")}
        hint={t("favorites.biliCookieHint")}
        placeholder={t("favorites.biliCookiePlaceholder")}
        note={t("favorites.biliExperimental")}
        multiline
        onSaved={setBiliConfigured}
      />

      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />

      {/* AI 检索助手：把「我想要个做 X 的库」翻译成检索动作，并把结果整理成清单 */}
      {aiOpen && (
        <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[620px] max-w-[95vw] max-h-[85vh] flex flex-col rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-3">
              <div className="w-9 h-9 rounded-xl bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
                <Bot className="w-4 h-4 text-[var(--module-accent)]" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">{t("favorites.aiSearchTitle")}</h3>
                <p className="text-[10px] text-slate-500">{t("favorites.aiSearchHint")}</p>
              </div>
              <button
                onClick={() => setAiOpen(false)}
                className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer"
              >
                ✕
              </button>
            </div>

            <div className="flex items-center gap-2 mb-2">
              <input
                value={aiInput}
                onChange={(e) => setAiInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void runAiSearch(); } }}
                placeholder={t("favorites.aiSearchPh")}
                className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
              />
              <button
                onClick={() => void runAiSearch()}
                disabled={aiBusy || !aiInput.trim()}
                className="px-3 py-2 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-90 text-white font-semibold cursor-pointer disabled:opacity-40 flex items-center gap-1"
              >
                <Send className="w-3 h-3" />
                {aiBusy ? t("favorites.aiSearchRunning") : t("favorites.aiSearchGo")}
              </button>
            </div>

            {/* 检索过程：让用户看得见 agent 到底查了什么，而不是黑箱等结果 */}
            {aiSteps.length > 0 && (
              <div ref={aiLogRef} className="max-h-24 overflow-y-auto rounded-lg border border-white/5 bg-black/30 p-2 mb-2 space-y-1">
                {aiSteps.map((s, i) => (
                  <div key={i} className="text-[9px] text-slate-500 flex gap-1.5">
                    <span className="flex-shrink-0 text-slate-600">{AI_TOOL_LABEL[s.tool ?? ""] ?? "过程"}</span>
                    <span className="min-w-0 break-all">{s.text.slice(0, 160)}</span>
                  </div>
                ))}
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-white/5 bg-slate-900/30 p-3">
              {aiError ? (
                <div className="text-[11px] text-rose-400 break-all">{aiError}</div>
              ) : aiResult ? (
                <AiResultMarkdown text={aiResult} />
              ) : (
                <div className="text-[11px] text-slate-500 py-8 text-center">
                  <Sparkles className="w-4 h-4 mx-auto mb-2 text-slate-600" />
                  {t("favorites.aiSearchPlaceholder")}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* 分类右键菜单 */}
      {catMenu && (
        <div
          className="fixed z-[300] bg-surface-panel border border-white/15 rounded-lg shadow-2xl py-1 text-[11px] min-w-[140px]"
          style={{ left: catMenu.x, top: catMenu.y }}
          onMouseLeave={() => setCatMenu(null)}
        >
          <div className="px-2 py-1 text-slate-500 truncate border-b border-white/5 mb-1">
            {catMenu.name}
          </div>
          <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
            onClick={() => { const id = catMenu.id; setCatMenu(null); void createCat(id); }}>
            {t("favorites.newSubCategory")}
          </button>
          {catMenu.id != null && (
            <>
              <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void renameCat(id); }}>
                {t("favorites.renameCategory")}
              </button>
              <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void moveCat(id, -1); }}>
                {t("favorites.moveUp")}
              </button>
              <button className="w-full text-left px-2 py-1 hover:bg-white/10 text-slate-200 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void moveCat(id, 1); }}>
                {t("favorites.moveDown")}
              </button>
              <button className="w-full text-left px-2 py-1 hover:bg-rose-500/15 text-rose-300 cursor-pointer"
                onClick={() => { const id = catMenu.id!; setCatMenu(null); void deleteCat(id); }}>
                {t("favorites.deleteCategory")}
              </button>
            </>
          )}
        </div>
      )}

      {/* 条目分类选择器：勾选式，支持多级 */}
      {pickerFor && (
        <div className="fixed inset-0 z-[300] modal-mask bg-black/60 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="w-full max-w-sm bg-surface-panel border border-white/15 rounded-2xl p-4 shadow-2xl space-y-3 text-xs">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-bold text-white">{t("favorites.pickCategoryTitle")}</h3>
              <button onClick={() => setPickerFor(null)} className="text-slate-400 hover:text-white p-1">
                <X className="w-4 h-4" />
              </button>
            </div>
            <p className="text-[10px] text-slate-500 truncate">{pickerFor.title}</p>
            <div className="max-h-64 overflow-y-auto space-y-0.5">
              {cats.length === 0 && (
                <p className="text-[11px] text-slate-500">{t("favorites.noTagsHint")}</p>
              )}
              <CategoryCheckTree
                nodes={cats}
                depth={0}
                selected={pickerSelected}
                onToggle={(id) => setPickerSelected((s) =>
                  s.includes(id) ? s.filter((x) => x !== id) : [...s, id]
                )}
              />
            </div>
            <div className="flex items-center gap-2 pt-1">
              <SharedButton className="!h-7 !px-3 !text-[11px]" onClick={() => void submitPicker()}>
                {t("common.save")}
              </SharedButton>
              <button onClick={() => setPickerFor(null)}
                className="px-3 py-1 rounded-lg bg-white/5 hover:bg-white/10 text-[11px] text-slate-300 cursor-pointer">
                {t("common.cancel")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** 选择器里的勾选树（缩进 + 复选框）。 */
function CategoryCheckTree({
  nodes, depth, selected, onToggle,
}: {
  nodes: FavoriteCategoryNode[];
  depth: number;
  selected: number[];
  onToggle: (id: number) => void;
}) {
  return (
    <>
      {nodes.map((c) => (
        <div key={c.id}>
          <label
            className="flex items-center gap-1.5 py-1 rounded hover:bg-white/5 cursor-pointer"
            style={{ paddingLeft: depth * 12 }}
          >
            <input
              type="checkbox"
              checked={selected.includes(c.id)}
              onChange={() => onToggle(c.id)}
              className="accent-[var(--module-accent)] cursor-pointer"
            />
            <span className="text-[11px] text-slate-200 truncate">{c.name}</span>
            <span className="ml-auto text-[10px] text-slate-500">{c.count}</span>
          </label>
          {c.children.length > 0 && (
            <CategoryCheckTree
              nodes={c.children}
              depth={depth + 1}
              selected={selected}
              onToggle={onToggle}
            />
          )}
        </div>
      ))}
    </>
  );
}

/** 分类树（递归）：缩进表示层级，箭头展开/折叠，右键出操作菜单。
 *  操作逻辑对齐启动模块的分类：点选筛选（含子分类）、右键改名/新建子分类/上下移/删除。 */
function CategoryTree({
  nodes, depth, expanded, selectedId, onToggle, onSelect, onContextMenu,
}: {
  nodes: FavoriteCategoryNode[];
  depth: number;
  expanded: Set<number>;
  selectedId: number | null;
  onToggle: (id: number) => void;
  onSelect: (id: number, name: string) => void;
  onContextMenu: (id: number, name: string, x: number, y: number) => void;
}) {
  return (
    <>
      {nodes.map((c) => {
        const hasKids = c.children.length > 0;
        const open = expanded.has(c.id);
        return (
          <div key={c.id}>
            <div
              className={`flex items-center gap-0.5 rounded text-[11px] cursor-pointer transition-colors ${
                selectedId === c.id
                  ? "bg-[var(--module-accent-soft)] text-white"
                  : "text-slate-400 hover:bg-white/5"
              }`}
              style={{ paddingLeft: 4 + depth * 10 }}
              onClick={() => onSelect(c.id, c.name)}
              onContextMenu={(e) => {
                e.preventDefault();
                onContextMenu(c.id, c.name, e.clientX, e.clientY);
              }}
              title={c.name}
            >
              <button
                onClick={(e) => { e.stopPropagation(); if (hasKids) onToggle(c.id); }}
                className={`w-3 shrink-0 text-slate-600 hover:text-slate-300 ${hasKids ? "cursor-pointer" : "invisible"}`}
              >
                {open ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
              </button>
              <span className="truncate flex-1 py-1">{c.name}</span>
              <span className="pr-1 text-slate-500">{c.total}</span>
            </div>
            {open && hasKids && (
              <CategoryTree
                nodes={c.children}
                depth={depth + 1}
                expanded={expanded}
                selectedId={selectedId}
                onToggle={onToggle}
                onSelect={onSelect}
                onContextMenu={onContextMenu}
              />
            )}
          </div>
        );
      })}
    </>
  );
}

/** 极简 Markdown 渲染：只处理 agent 输出约定里的几种行（标题 / 列表项 / 普通段落）。
 *  不引第三方渲染库——内容是可控的模型输出，够用且没有 XSS 面（链接走 openUrl）。 */
function AiResultMarkdown({ text }: { text: string }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5">
      {text.split("\n").map((raw, i) => {
        const parsed = parseAiResultLine(raw);
        switch (parsed.kind) {
          case "blank":
            return <div key={i} className="h-1" />;
          case "heading":
            return parsed.level === 2 ? (
              <div key={i} className="text-xs font-bold text-white mt-2">{parsed.text}</div>
            ) : (
              <div key={i} className="text-[11px] font-bold text-slate-200 mt-2">{parsed.text}</div>
            );
          case "item":
            return (
              <div key={i} className="flex items-start gap-1.5 text-[11px]">
                <span className="text-slate-600 mt-[3px]">•</span>
                <button
                  onClick={() => { void openUrl(parsed.url).catch(() => {}); }}
                  className="text-[var(--module-accent)] hover:underline cursor-pointer text-left"
                  title={parsed.url}
                >
                  {parsed.title}
                </button>
                {parsed.note && <span className="text-slate-400">{parsed.note}</span>}
              </div>
            );
          case "bullet":
            return <div key={i} className="text-[11px] text-slate-300 pl-2">• {parsed.text}</div>;
          default:
            return (
              <div key={i} className="text-[11px] text-slate-300 whitespace-pre-wrap">{parsed.text}</div>
            );
        }
      })}
      <div className="pt-2 text-[9px] text-slate-600">{t("favorites.aiSearchDisclaimer")}</div>
    </div>
  );
}

const AI_TOOL_LABEL: Record<string, string> = {
  search_favorites: "检索",
  get_favorite_content: "读正文",
  list_favorite_tags: "看分类",
};

/** 分组之间的细分隔线（比留白更明确地「断开」，又不至于像卡片那样围起来）。 */
function Divider() {
  return <span className="w-px h-3.5 bg-white/10" />;
}

/**
 * 工具栏的文字链接按钮。
 *
 * 刻意做成纯文字：这一排全是同级动作，用按钮样式会把「导入」这种高频操作
 * 和其它动作拉成一样的视觉重量，反而看不出主次，一行里还会挤满色块。
 * `busy` 时显示转圈并禁用点击（避免重复触发）。
 */
function LinkButton({
  label,
  onClick,
  disabled,
  busy,
  danger,
  title,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  busy?: boolean;
  danger?: boolean;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || busy}
      title={title}
      className={`inline-flex items-center gap-1 text-[11px] cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
        danger
          ? "text-rose-400/80 hover:text-rose-300"
          : "text-slate-400 hover:text-[var(--module-accent)]"
      }`}
    >
      {busy && <RefreshCw className="w-2.5 h-2.5 animate-spin" />}
      {label}
    </button>
  );
}

/** 「密钥」链接：已配置为正常色，未配置更暗；有告警时带一个点。 */
function KeyLink({
  label,
  configured,
  title,
  alert,
  onClick,
}: {
  label: string;
  configured: boolean;
  title: string;
  alert?: "expired" | "soon" | null;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`relative text-[11px] cursor-pointer transition-colors ${
        configured
          ? "text-slate-400 hover:text-[var(--module-accent)]"
          : "text-slate-600 hover:text-slate-300"
      }`}
    >
      {label}
      <CredDot level={alert ?? null} />
    </button>
  );
}

/**
 * 凭证链接右上角的告警点。
 *
 * 只做「一眼看出有事」，具体原因交给 `title`——因为这里能承载的信息量太小，
 * 把「3 天后过期」塞进一个点里只会让人困惑。
 */
function CredDot({ level }: { level: "expired" | "soon" | null }) {
  if (!level) return null;
  return (
    <span
      className={`absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full ${
        level === "expired" ? "bg-rose-400" : "bg-amber-400"
      }`}
    />
  );
}
