import { FavoriteDeletedDialog } from "./FavoriteDeletedDialog";
import { FavoriteLibrarySummary } from "./FavoriteLibrarySummary";
import { FavoriteTaskProgress } from "./FavoriteTaskProgress";
import { FavoriteItemFilters } from "./FavoriteItemFilters";
import { useFavoriteTasks } from "./useFavoriteTasks";
import { useFavoriteAiAssistant } from "./useFavoriteAiAssistant";
import { useFavoriteCategories } from "./useFavoriteCategories";
import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, Clock, ExternalLink, Pencil, RefreshCw, Trash2, Star } from "lucide-react";
import { favoritedDateLabel, sinceToLocalString, type SincePreset } from "./favoritedTime";
import { Menu } from "../shared/Menu";
import { useVirtualList } from "../shared/useVirtualList";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { toast } from "../shared/Toast";
import VexEmptyState from "../VexEmptyState";
import { GithubTokenDialog } from "../project/GithubTokenDialog";
import { MarkdownRenderer } from "../shared/markdown/MarkdownRenderer";
import { CredentialDialog } from "./CredentialDialog";
import type { AiConfig, AiProvider } from "../../contracts/ai";
import { ModelSelector } from "../shared/ai/ModelSelector";
import { SOURCE_LABELS, expiringInDays, statusBadge, type CachedContent, type CredentialStatus, type FavoriteDeletedRow, type FavoriteRow, type FavoriteSettings, type FavoriteStats } from "./types";
import { DEFAULT_AGENT_ROUNDS, DEFAULT_AI_WIDTH, DEFAULT_LEFT_WIDTH, MAX_AGENT_ROUNDS, MAX_AI_WIDTH, MAX_LEFT_WIDTH, MIN_AGENT_ROUNDS, MIN_AI_WIDTH, MIN_LEFT_WIDTH, isImportTask } from "./FavoritesPanel.model";
import { CredDot } from "./FavoritesPanel.CredDot";
import { LinkButton } from "./FavoritesPanel.LinkButton";
import { Divider } from "./FavoritesPanel.Divider";

export default function FavoritesPanel() {
  const { t } = useTranslation();

  const [items, setItems] = useState<FavoriteRow[]>([]);
  const [stats, setStats] = useState<FavoriteStats | null>(null);
  // 分类筛选：走分类 id（树），名字只用于回显

  const [source, setSource] = useState<string | null>(null);
  const [keyword, setKeyword] = useState("");
  // 收藏时间过滤（全部 / 近 7 天 / 近 30 天 / 近一年）
  const [since, setSince] = useState<SincePreset>("all");
  // 正在跑的长任务（替换原先单个 busy 字段）：三个导入可以同时在列，
  // 归类 / 失效检测与导入互斥，所以加工类任务在列时导入按钮全灰。

  // AI 归类的模型选择：直接复用 AI 模块的配置，不另设一套
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  const {
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
    tokenConfigured,
    setTokenConfigured,
    tokenOpen,
    setTokenOpen,
    runImport,
    runImportBili,
    runImportZhihu,
    runImportBookmarks,
    runClassify,
    runCheck,
    requestStop,
  } = useFavoriteTasks({
    refresh: () => refresh(),
    providerId,
    modelId,
  });

  // 左侧分类栏宽度：可拖动，宽度与模型选择一起存进 favorites_settings.json
  const [leftWidth, setLeftWidth] = useState(DEFAULT_LEFT_WIDTH);
  // AI 检索右栏（常驻，不再是弹窗）：宽度可拖动，显隐与宽度一起持久化
  const [aiWidth, setAiWidth] = useState(DEFAULT_AI_WIDTH);
  // 设置的内存副本：拖动/切模型都是「读-改-写」，先攒在这里再整份落盘
  // 检索 Agent 的轮数上限：每轮一次模型调用，调大更会找但更费 token（后端再钳一次 1..20）
  const [agentRounds, setAgentRounds] = useState(DEFAULT_AGENT_ROUNDS);
  const settingsRef = useRef<FavoriteSettings>({
    leftWidth: DEFAULT_LEFT_WIDTH,
    providerId: null,
    modelId: null,
    agentRounds: DEFAULT_AGENT_ROUNDS,
    aiWidth: DEFAULT_AI_WIDTH,
    aiOpen: true,
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
  const {
    setAiResult,
    setAiSteps,
    aiBusy,
    setAiError,
    renderAiAssistant,
  } = useFavoriteAiAssistant({
    providerId,
    modelId,
    aiWidth,
    toggleAiPanel: () => toggleAiPanel(),
  });

  /** 条目列表虚拟化：只渲染视口内的行（行高由实测修正，展开正文后也会跟着变）。 */
  const vlist = useVirtualList({
    items,
    getKey: (item) => String(item.id),
    // 未测量前的估计值：标题 + 描述两行 + 标签行 ≈ 76px
    estimateHeight: 76,
  });

  // 凭证健康状态：Cookie 快过期/已失效时主动提醒
  const [credStatus, setCredStatus] = useState<CredentialStatus[]>([]);
  const [credWarned, setCredWarned] = useState(false);

  // 展开看正文：知乎收藏内容 / GitHub README（懒抓，抓过就缓存在库里）
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [content, setContent] = useState<Record<number, CachedContent | null>>({});
  const [contentBusy, setContentBusy] = useState<number | null>(null);

  const {
    categoryId,
    setCategoryId,
    categoryName,
    setCategoryName,
    starredOnly,
    catMenu,
    pickerFor,
    organizeOpen,
    findCatByName,
    openPicker,
    renderCategorySidebar,
    renderCategoryMenu,
    renderOrganizeDialog,
    renderCategoryPicker,
  } = useFavoriteCategories({
    stats,
    refresh: () => refresh(),
    providerId,
    modelId,
    running,
    leftWidth,
    starredCount: items.filter(item => item.isStarred).length,
  });

  const refresh = useCallback(async () => {
    const [list, overview, creds, deleted] = await Promise.all([
      invoke<FavoriteRow[]>("fav_list", {
        source,
        // 分类改成树之后按 id 筛（后端会把子分类一起算进来）
        categoryId: categoryId ?? undefined,
        status: null,
        keyword: keyword.trim() || null,
        sort: "favorited",
        // 时区换算放在前端：后端只拿到一个「不早于」的本地时间串，不需要猜时区
        favoritedSince: sinceToLocalString(since),
        starredOnly: starredOnly || null,
        limit: 0,
      }),
      invoke<FavoriteStats>("fav_stats"),
      invoke<CredentialStatus[]>("fav_credential_status"),
      invoke<FavoriteDeletedRow[]>("fav_list_deleted"),
    ]);
    setItems(list);
    setStats(overview);
    setCredStatus(creds);
    setDeletedRows(deleted ?? []);
  }, [source, categoryId, keyword, since, starredOnly]);

  // 星标数量：当前列表里的（分类/关键词筛选会收窄它，与列表所见保持一致）

  /** 星标开关：乐观更新（先改 UI 再发请求），失败时回滚并提示。
   *  星标是纯本地标记，不涉及任何平台同步，所以不该有等待感。 */
  const toggleStar = async (item: FavoriteRow) => {
    const next = !item.isStarred;
    setItems((cur) => cur.map((x) => (x.id === item.id ? { ...x, isStarred: next } : x)));
    try {
      await invoke("fav_set_starred", { id: item.id, starred: next });
    } catch (e) {
      setItems((cur) => cur.map((x) => (x.id === item.id ? { ...x, isStarred: item.isStarred } : x)));
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

  // 供应商/模型：**只**恢复收藏模块自己上次选的，不做任何兜底。
  //
  // 以前会依次回落「全局默认（翻译模块选的）→ 首个可用供应商 → 该供应商的激活模型」，
  // 结果是界面看着像没选、实际却跑在别的账号/模型上。现在宁可为空（按钮禁用 + 提示先选），
  // 也不替用户做决定。
  useEffect(() => {
    let alive = true;
    void Promise.all([
      invoke<AiConfig>("get_ai_config"),
      invoke<FavoriteSettings>("fav_get_settings").catch(() => null),
    ])
      .then(([cfg, saved]) => {
        if (!alive) return;
        const all = cfg.providers || [];
        // 列表里只放「能用于归类」的：有 OpenAI 兼容端点 + Key
        const usable = all.filter((p) => p.openai_url && p.api_key);
        setProviders(usable);

        // 分类栏宽度：坏值回默认，越界收敛（后端也会再钳一次）
        const savedWidth = saved?.leftWidth;
        const width =
          typeof savedWidth === "number" && Number.isFinite(savedWidth) && savedWidth > 0
            ? Math.min(MAX_LEFT_WIDTH, Math.max(MIN_LEFT_WIDTH, savedWidth))
            : DEFAULT_LEFT_WIDTH;
        setLeftWidth(width);

        // AI 检索右栏：宽度同样钳制，显隐按上次的来（默认展开）
        const savedAiWidth = saved?.aiWidth;
        const aiW =
          typeof savedAiWidth === "number" && Number.isFinite(savedAiWidth) && savedAiWidth > 0
            ? Math.min(MAX_AI_WIDTH, Math.max(MIN_AI_WIDTH, savedAiWidth))
            : DEFAULT_AI_WIDTH;
        setAiWidth(aiW);
        setAiOpen(saved?.aiOpen ?? true);
        const rounds =
          typeof saved?.agentRounds === "number" && Number.isFinite(saved.agentRounds)
            ? Math.min(MAX_AGENT_ROUNDS, Math.max(MIN_AGENT_ROUNDS, saved.agentRounds))
            : DEFAULT_AGENT_ROUNDS;
        setAgentRounds(rounds);
        settingsRef.current = {
          leftWidth: width,
          providerId: saved?.providerId ?? null,
          modelId: saved?.modelId ?? null,
          agentRounds: rounds,
          aiWidth: aiW,
          aiOpen: saved?.aiOpen ?? true,
        };

        // 上次选的供应商还在可用列表里 → 恢复；否则留空等用户选
        const savedProvider =
          saved?.providerId && usable.find((p) => p.id === saved.providerId);
        if (!savedProvider) return;
        setProviderId(savedProvider.id);
        // 模型同样只在「仍属于该供应商」时恢复
        const savedModel =
          saved?.modelId &&
          savedProvider.models.some((m) => m.id === saved.modelId)
            ? saved.modelId
            : "";
        setModelId(savedModel);
      })
      .catch(() => setProviders([]));
    return () => {
      alive = false;
    };
  }, []);

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

  /** AI 检索右栏宽度拖动（方向相反：往左拖变宽） */
  const startAiResize = (e: ReactMouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = aiWidth;
    let next = startWidth;
    const onMove = (ev: MouseEvent) => {
      next = Math.min(MAX_AI_WIDTH, Math.max(MIN_AI_WIDTH, startWidth - (ev.clientX - startX)));
      setAiWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      if (next !== startWidth) void persistSettings({ aiWidth: next });
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  /** 开关 AI 检索右栏（显隐持久化） */
  const toggleAiPanel = () => {
    setAiOpen((prev) => {
      const next = !prev;
      void persistSettings({ aiOpen: next });
      return next;
    });
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

  // ── 删除墓碑：删过的条目不该被下次导入捞回来，但用户可以「重新纳入」──
  const [deletedRows, setDeletedRows] = useState<FavoriteDeletedRow[]>([]);
  const [deletedOpen, setDeletedOpen] = useState(false);

  const restoreDeleted = async (source?: string, externalId?: string) => {
    try {
      const n = await invoke<number>("fav_restore_deleted", {
        source: source ?? null,
        externalId: externalId ?? null,
      });
      toast(t("favorites.restoreDeletedDone", { count: n }), "ok");
      await refresh();
    } catch (e) {
      toast(t("favorites.restoreDeletedFail", { err: String(e) }), "err");
    }
  };

  /** 任一凭证有告警（过期/快过期）→ 导入菜单上点一个点，别让告警只藏在菜单里。 */
  const credLevel: "expired" | "soon" | null = (() => {
    const levels = ["bilibili", "zhihu-cookie"].map((k) => credAlert(k));
    if (levels.includes("expired")) return "expired";
    if (levels.includes("soon")) return "soon";
    return null;
  })();

  return (
    <div className="h-full flex flex-col gap-2 p-3 text-slate-200">
      {/* 顶部操作区：**一行放得下**才算整齐。
          按来源平铺「GitHub / 浏览器 / B站 / 知乎 / 工具」五组必然自动换行成两排
          且每加一个来源就多挤一格，所以收成两个菜单：
          导入（各来源 + 各自的密钥）与 归类（供应商 / 模型 + 开始归类）。
          跑着的导入会在菜单外露出「停止」，那是当下最该点的东西，不能藏进菜单。 */}
      <div className="flex items-center gap-2 flex-nowrap min-w-0">
        <Menu
          label={
            <span className="relative" title={credLevel ? t("favorites.credExpiredHintShort") : undefined}>
              {t("favorites.importMenu")}
              <CredDot level={credLevel} />
            </span>
          }
          busy={importRunning}
          disabled={processRunning}
          title={processRunning ? t("favorites.importBlockedByProcess") : t("favorites.importMenuHint")}
          items={[
            {
              key: "github",
              label: "GitHub star",
              active: running.includes("github"),
              hint: running.includes("github")
                ? t("favorites.importingShort")
                : tokenConfigured
                  ? t("favorites.keyReady")
                  : t("favorites.keyMissing"),
              disabled: running.includes("github") || processRunning,
              onSelect: () => void runImport(),
            },
            {
              key: "edge",
              label: t("favorites.edgeBookmarks"),
              active: running.includes("bookmark"),
              hint: running.includes("bookmark") ? t("favorites.importingShort") : undefined,
              disabled: running.includes("bookmark") || processRunning,
              onSelect: () => void runImportBookmarks("edge"),
            },
            {
              key: "chrome",
              label: t("favorites.chromeBookmarks"),
              active: running.includes("bookmark"),
              disabled: running.includes("bookmark") || processRunning,
              onSelect: () => void runImportBookmarks("chrome"),
            },
            {
              key: "bilibili",
              label: t("favorites.biliFavorites"),
              active: running.includes("bilibili"),
              hint: running.includes("bilibili")
                ? t("favorites.importingShort")
                : biliConfigured
                  ? t("favorites.keyReady")
                  : t("favorites.keyMissing"),
              disabled: running.includes("bilibili") || processRunning,
              onSelect: () => (biliConfigured ? void runImportBili() : setCookieOpen(true)),
            },
            {
              key: "zhihu",
              label: t("favorites.zhihuFavorites"),
              active: running.includes("zhihu"),
              hint: running.includes("zhihu")
                ? t("favorites.importingShort")
                : zhihuCookieConfigured
                  ? t("favorites.keyReady")
                  : t("favorites.keyMissing"),
              disabled: running.includes("zhihu") || processRunning,
              onSelect: () => void runImportZhihu(),
            },
            { key: "sep-keys", type: "separator" },
            { key: "hdr-keys", type: "header", label: t("favorites.keysHeader") },
            // 密钥**永远打开弹窗**（回显已存的 Cookie）：配过就直接跑的写法
            // 让用户再也进不去弹窗，改不了一份过期 Cookie。
            {
              key: "key-github",
              label: t("favorites.githubTokenShort"),
              hint: tokenConfigured ? t("favorites.keyReady") : t("favorites.keyMissing"),
              onSelect: () => setTokenOpen(true),
            },
            {
              key: "key-bilibili",
              label: t("favorites.biliCookieShort"),
              hint: biliConfigured ? t("favorites.keyReady") : t("favorites.keyMissing"),
              active: credAlert("bilibili") != null,
              onSelect: () => setCookieOpen(true),
            },
            {
              key: "key-zhihu",
              label: t("favorites.zhihuCookieShort"),
              hint: zhihuCookieConfigured ? t("favorites.keyReady") : t("favorites.keyMissing"),
              active: credAlert("zhihu-cookie") != null,
              onSelect: () => setZhihuCookieOpen(true),
            },
          ]}
        />

        {/* 跑着的导入：各自一个「停止」，直接露在外面 */}
        {running.filter(isImportTask).map((task) => (
          <LinkButton key={task} label={t("favorites.stop")} danger onClick={() => void requestStop(task)} />
        ))}

        <Divider />

        {/* 归类用的供应商 / 模型：收藏模块**自己的**选项，直接摆在顶栏。
            两处都不设「跟随 AI 模块默认」——归类跑在哪个账号、哪个模型上必须一眼可辨，
            后端也不再兜底（缺任一项直接报错，见 pick_explicit_provider/model）。 */}
        <div className="w-[210px] max-w-[210px]">
          <ModelSelector
            providers={providers}
            providerId={providerId}
            modelId={modelId}
            onSelect={(pid, mid) => {
              setProviderId(pid);
              setModelId(mid);
              void persistSettings({ providerId: pid || null, modelId: mid || null });
            }}
            placeholder={t("favorites.providerPick")}
            emptyText={t("favorites.providerPick")}
            compact
          />
        </div>

        {/* 检索 Agent 轮数：只影响「AI 检索」，归类不走这个循环 */}
        <input
          type="number"
          min={MIN_AGENT_ROUNDS}
          max={MAX_AGENT_ROUNDS}
          value={agentRounds}
          onChange={(e) => {
            const raw = Number(e.target.value);
            const next = Number.isFinite(raw)
              ? Math.min(MAX_AGENT_ROUNDS, Math.max(MIN_AGENT_ROUNDS, Math.round(raw)))
              : DEFAULT_AGENT_ROUNDS;
            setAgentRounds(next);
            void persistSettings({ agentRounds: next });
          }}
          className="glass-input px-1.5 h-6 w-12 text-caption text-center"
          title={t("favorites.agentRoundsHint")}
        />

        <Menu
          label={t("favorites.classify")}
          busy={running.includes("classify")}
          disabled={running.length > 0 || !providerId || !modelId}
          title={
            !providerId || !modelId
              ? t("favorites.needProviderModel")
              : importRunning
                ? t("favorites.processBlockedByImport")
                : t("favorites.classifyHint")
          }
          items={[
            {
              key: "unclassified",
              label: t("favorites.classifyUnclassified"),
              hint: stats?.unclassified != null ? String(stats.unclassified) : undefined,
              onSelect: () => void runClassify(false),
            },
            {
              key: "reclassify",
              label: t("favorites.classifyReclassify"),
              onSelect: () => void runClassify(true),
            },
          ]}
        />

        <LinkButton
          label={t("favorites.check")}
          busy={running.includes("check")}
          disabled={running.length > 0}
          title={importRunning ? t("favorites.processBlockedByImport") : t("favorites.checkHint")}
          onClick={() => void runCheck()}
        />
        {/* AI 检索：常驻右栏（点一下展开/收起，不再弹窗）；输入框在右栏里 */}
        <LinkButton
          label={aiOpen ? t("favorites.aiSearchHide") : t("favorites.aiSearch")}
          active={aiOpen}
          busy={aiBusy}
          disabled={running.length > 0 || !providerId || !modelId}
          title={
            !providerId || !modelId
              ? t("favorites.needProviderModel")
              : importRunning
                ? t("favorites.processBlockedByImport")
                : undefined
          }
          onClick={() => {
            if (aiOpen) { toggleAiPanel(); return; }
            setAiError(null);
            setAiResult("");
            setAiSteps([]);
            toggleAiPanel();
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

        <FavoriteItemFilters
          keyword={keyword}
          setKeyword={setKeyword}
          source={source}
          setSource={setSource}
          stats={stats}
          since={since}
          setSince={setSince}
        />
      </div>

      {/* 实时进度：**每个在跑的任务一行** —— 三个导入可以同时跑，各显示各的，
          用一条进度条会让后到的事件盖掉前一个。任务结束由 done 事件（或 finally）清掉。 */}
      {progressRows.length > 0 && (
        <FavoriteTaskProgress
          progressRows={progressRows}
        />
      )}

      <div className="flex-1 flex min-h-0">
        {/* 左侧分类树（宽度可拖动，默认 180px，拖动后写回设置） */}
        {renderCategorySidebar()}

        {/* 分隔条：左右布局的唯一调节点（拖动改左栏宽度） */}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t("favorites.resizeLeftTip")}
          title={t("favorites.resizeLeftTip")}
          onMouseDown={startLeftResize}
          className="w-2 shrink-0 cursor-col-resize rounded transition-colors hover:bg-[var(--module-accent-soft)]"
        />

        {/* 右侧条目列表：虚拟滚动 —— 只挂载视口内的行。
            收藏动辄几千条（star + 书签 + B站 + 知乎），全量 DOM 会拖慢输入与滚动；
            行高不定（描述换行 / 展开正文），由 useVirtualList 实测后修正。 */}
        <div
          ref={vlist.scrollRef}
          onScroll={vlist.onScroll}
          className="flex-1 overflow-y-auto glass-panel relative"
        >
          {items.length === 0 && (
            <VexEmptyState
              title={t("favorites.empty")}
              tick={t("favorites.emptyTick")}
              avatarSize={44}
              className="h-full"
            />
          )}
          <div className="relative" style={{ height: vlist.totalHeight }}>
          {vlist.visible.map(({ item, key, index }) => {
            const badge = statusBadge(item.status);
            return (
              <div
                key={key}
                ref={vlist.measureRef(key)}
                // 行内边距走密度 token（行高由 useVirtualList 实测，改密度不需要改测量逻辑）
                className="absolute left-0 right-0 top-0 px-3 py-[var(--density-py)] hover:bg-white/5 group border-b border-white/5"
                style={{ transform: `translateY(${vlist.offsets[index]}px)` }}
              >
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <button
                        onClick={() => void openUrl(item.url).catch(() => toast(item.url, "err"))}
                        className="text-body font-medium text-slate-100 hover:text-[var(--module-accent)] cursor-pointer truncate flex items-center gap-1"
                        title={item.url}
                      >
                        {item.title}
                        <ExternalLink className="w-2.5 h-2.5 opacity-0 group-hover:opacity-60" />
                      </button>
                      <span className="text-micro px-1 rounded bg-white/5 text-slate-500">
                        {SOURCE_LABELS[item.source] ?? item.source}
                      </span>
                      {/* 收藏时间：只显示平台记录的时间；没有就不显示 */}
                      {(() => {
                        const platform = favoritedDateLabel(item.favoritedAt);
                        if (!platform) return null;
                        return (
                          <span
                            className="text-micro px-1 rounded bg-white/5 text-slate-500 flex items-center gap-0.5 shrink-0"
                            title={t("favorites.favoritedAtTip", { time: item.favoritedAt })}
                          >
                            <Clock className="w-2.5 h-2.5" />
                            {platform}
                          </span>
                        );
                      })()}
                      {badge && (
                        <span className={`text-micro px-1 rounded ${badge.className}`}>
                          {t(badge.text)}
                        </span>
                      )}
                      {item.aiLocked && (
                        <span
                          className="text-micro px-1 rounded bg-emerald-500/10 text-emerald-400/80"
                          title={t("favorites.lockedHint")}
                        >
                          {t("favorites.locked")}
                        </span>
                      )}
                    </div>
                    {item.description && (
                      <p className="text-tiny text-slate-400 line-clamp-2 mt-0.5">
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
                            className="text-micro px-1.5 py-0.5 rounded-full bg-white/5 text-slate-300 hover:bg-white/10 cursor-pointer"
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
                      onClick={() => void toggleStar(item)}
                      className={`p-1 rounded cursor-pointer ${
                        item.isStarred
                          ? "text-amber-400"
                          : "text-slate-600 hover:text-amber-400 opacity-0 group-hover:opacity-100"
                      }`}
                      title={item.isStarred ? t("favorites.unstar") : t("favorites.star")}
                    >
                      <Star className={`w-3 h-3 ${item.isStarred ? "fill-current" : ""}`} />
                    </button>
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
                  <div className="mt-1.5 rounded-ctl border border-white/10 bg-black/25 p-2">
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-micro text-slate-500 truncate">
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
                          className="ml-auto text-micro text-slate-500 hover:text-slate-200 cursor-pointer disabled:opacity-40"
                          title={t("favorites.contentRefresh")}
                        >
                          {t("favorites.contentRefresh")}
                        </button>
                      )}
                    </div>
                    {contentBusy === item.id ? (
                      <div className="flex items-center gap-2 text-tiny text-slate-500 py-2">
                        <RefreshCw className="w-3 h-3 animate-spin" />
                        {t("favorites.contentLoading")}
                      </div>
                    ) : content[item.id]?.text ? (
                      <div className="max-h-72 overflow-y-auto">
                        {item.source === "github" ? (
                          <MarkdownRenderer content={content[item.id]!.text} />
                        ) : (
                          // 知乎正文只渲染纯文本：远端 HTML 直接进 DOM 等于把注入面交给知乎
                          <p className="text-caption text-slate-300 leading-relaxed whitespace-pre-wrap">
                            {content[item.id]!.text}
                          </p>
                        )}
                      </div>
                    ) : (
                      <p className="text-tiny text-slate-500 py-1">
                        {t("favorites.contentEmpty")}
                      </p>
                    )}
                  </div>
                )}
              </div>
            );
          })}
          </div>

          {/* AI 检索助手：常驻右栏（原先是全屏弹窗，一开就挡住列表）。
              与条目列表同屏，边看结果边翻收藏；宽度可拖、可关，状态持久化。 */}
          {aiOpen && (
            <>
              <div
                role="separator"
                aria-orientation="vertical"
                aria-label={t("favorites.resizeAiTip")}
                title={t("favorites.resizeAiTip")}
                onMouseDown={startAiResize}
                className="w-2 shrink-0 cursor-col-resize rounded transition-colors hover:bg-[var(--module-accent-soft)]"
              />
              {renderAiAssistant()}
            </>
          )}
        </div>
      </div>

      <FavoriteLibrarySummary
        stats={stats}
        goneCount={goneCount}
        categoryId={categoryId}
        categoryName={categoryName}
        setCategoryId={setCategoryId}
        setCategoryName={setCategoryName}
        deletedRows={deletedRows}
        setDeletedOpen={setDeletedOpen}
      />

      {/* 已删除条目（墓碑）：重新纳入后，下次导入会把它们再拉进来 */}
      {deletedOpen && (
        <FavoriteDeletedDialog
          setDeletedOpen={setDeletedOpen}
          deletedRows={deletedRows}
          restoreDeleted={restoreDeleted}
        />
      )}

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

      {/* 分类右键菜单 */}
      {catMenu && (
        renderCategoryMenu()
      )}

      {/* 整理类目方案预览：AI 只给「改上级」建议，勾选后才应用；ID / 名称 / 条目关联都不动 */}
      {organizeOpen && (
        <div className="fixed inset-0 z-[300] modal-mask bg-black/60 backdrop-blur-sm flex items-center justify-center p-4">
          {renderOrganizeDialog()}
        </div>
      )}

      {/* 条目分类选择器：勾选式，支持多级 */}
      {pickerFor && (
        <div className="fixed inset-0 z-[300] modal-mask bg-black/60 backdrop-blur-sm flex items-center justify-center p-4">
          {renderCategoryPicker()}
        </div>
      )}
    </div>
  );
}
