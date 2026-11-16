import { useTranslation } from "react-i18next";

import { Menu, type MenuEntry } from "../shared/Menu";
import { CredDot } from "./FavoritesPanel.CredDot";
import type { FavoriteTasks } from "./useFavoriteTasks";

interface Props {
  /** useFavoriteTasks 的返回值：状态与各导入回调都在里面，避免再传十几个 props */
  tasks: FavoriteTasks;
  /** 凭证整体告警级别（决定「导入」左侧小圆点） */
  credLevel: "expired" | "soon" | null;
  /** 单个凭证的告警级别（决定密钥条目的高亮） */
  credAlert: (source: string) => "expired" | "soon" | null;
}

/**
 * 顶部「导入」菜单：各来源导入 + 各自的密钥。
 *
 * 抽成独立文件是因为主面板已到存量行数基线（AUDIT-049）：新增来源只能加在这里，
 * 不能再往 FavoritesPanel.tsx 里堆。
 */
export function FavoritesImportMenu({ tasks, credLevel, credAlert }: Props) {
  const { t } = useTranslation();
  const {
    running,
    importRunning,
    processRunning,
    biliConfigured,
    setCookieOpen,
    zhihuCookieConfigured,
    setZhihuCookieOpen,
    xCookieConfigured,
    setXCookieOpen,
    xQueryIdConfigured,
    setXQueryIdOpen,
    xOauth,
    setXOauthOpen,
    tokenConfigured,
    setTokenOpen,
    runImport,
    runImportBili,
    runImportZhihu,
    runImportX,
    runImportBookmarks,
  } = tasks;

  const items: MenuEntry[] = [
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
    {
      key: "x",
      label: t("favorites.xFavorites"),
      active: running.includes("x"),
      hint: running.includes("x")
        ? t("favorites.importingShort")
        : xOauth
          ? t("favorites.xOfficialReady", { name: xOauth.screenName || xOauth.name })
          : xCookieConfigured && xQueryIdConfigured
            ? t("favorites.xCookieReady")
            : t("favorites.keyMissing"),
      disabled: running.includes("x") || processRunning,
      onSelect: () => void runImportX(),
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
    {
      key: "key-x-oauth",
      label: t("favorites.xOauthShort"),
      hint: xOauth ? `@${xOauth.screenName}` : t("favorites.keyMissing"),
      active: credAlert("x-oauth") != null,
      onSelect: () => setXOauthOpen(true),
    },
    {
      key: "key-x-cookie",
      label: t("favorites.xCookieShort"),
      hint: xCookieConfigured ? t("favorites.keyReady") : t("favorites.keyMissing"),
      active: credAlert("x-cookie") != null,
      onSelect: () => setXCookieOpen(true),
    },
    {
      key: "key-x-query-id",
      label: t("favorites.xQueryIdShort"),
      hint: xQueryIdConfigured ? t("favorites.keyReady") : t("favorites.keyMissing"),
      onSelect: () => setXQueryIdOpen(true),
    },
  ];

  return (
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
      items={items}
    />
  );
}
