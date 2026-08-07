// Mihomo 后端命令封装（对齐 clash-party ipc.ts）
import { invoke } from "@tauri-apps/api/core";

export const mihomoApi = {
  // ---- state ----
  getState: () => invoke<any>("mihomo_get_state"),
  controllerInfo: () => invoke<any>("mihomo_controller_info"),

  // ---- core ----
  start: () => invoke("mihomo_start"),
  stop: () => invoke("mihomo_stop"),
  restart: () => invoke("mihomo_restart"),
  setCorePath: (path: string | null) =>
    invoke("mihomo_set_core_path", { path }),
  closeAllConnections: () => invoke("mihomo_close_all_connections"),

  // ---- app config ----
  getAppConfig: () => invoke<any>("mihomo_get_app_config"),
  patchAppConfig: (patch: any) =>
    invoke<any>("mihomo_patch_app_config", { patch }),

  // ---- controled mihomo config ----
  getControled: () => invoke<any>("mihomo_get_controled_config"),
  patchControled: (patch: any) =>
    invoke<any>("mihomo_patch_controled_config", { patch }),
  // 别名（与 clash-party 命名一致）
  getControledConfig: () => invoke<any>("mihomo_get_controled_config"),
  patchControledConfig: (patch: any) =>
    invoke<any>("mihomo_patch_controled_config", { patch }),

  // ---- runtime config ----
  getRuntimeConfig: () => invoke<string>("mihomo_get_runtime_config"),
  updateRuntimeConfig: () => invoke("mihomo_update_runtime_config"),

  // ---- profiles ----
  getProfileConfig: () => invoke<any>("mihomo_get_profile_config"),
  setProfileConfig: (cfg: any) =>
    invoke("mihomo_set_profile_config", { cfg }),
  getProfileItem: (id: string) =>
    invoke<any>("mihomo_get_profile_item", { id }),
  getProfileStr: (id: string) =>
    invoke<string>("mihomo_get_profile_str", { id }),
  setProfileStr: (id: string, content: string) =>
    invoke("mihomo_set_profile_str", { id, content }),
  addProfile: (item: any) => invoke("mihomo_add_profile", { item }),
  removeProfile: (id: string) =>
    invoke("mihomo_remove_profile", { id }),
  updateProfile: (item: any) => invoke("mihomo_update_profile", { item }),
  changeCurrentProfile: (id: string) =>
    invoke("mihomo_change_current_profile", { id }),
  validateSubscription: (url: string) =>
    invoke<any>("mihomo_validate_subscription", { url }),
  importSubscription: (url: string) =>
    invoke<any>("mihomo_import_subscription", { url }),
  importFile: (path: string) =>
    invoke<any>("mihomo_import_file", { path }),
  updateSubscription: (id: string) =>
    invoke<any>("mihomo_update_subscription", { id }),
  getProfileStatus: (id: string) =>
    invoke<any>("mihomo_get_profile_status", { id }),
  getProfileFilePath: (id: string) =>
    invoke<string>("mihomo_get_profile_file_path", { id }),

  // ---- overrides ----
  getOverrideConfig: () => invoke<any>("mihomo_get_override_config"),
  setOverrideConfig: (cfg: any) =>
    invoke("mihomo_set_override_config", { cfg }),
  getOverrideItem: (id: string) =>
    invoke<any>("mihomo_get_override_item", { id }),
  addOverride: (item: any) => invoke("mihomo_add_override", { item }),
  removeOverride: (id: string) =>
    invoke("mihomo_remove_override", { id }),
  updateOverride: (item: any) =>
    invoke("mihomo_update_override", { item }),
  getOverride: (id: string) => invoke<string>("mihomo_get_override", { id }),
  setOverride: (id: string, content: string) =>
    invoke("mihomo_set_override", { id, content }),

  // ---- controller proxy ----
  // 后端 mihomo_api 返回的是 controller 响应的 JSON 文本（String），这里统一解析为对象；
  // 非 JSON 响应（如 204 空 body）兜底返回原字符串，避免影响 changeProxy 等调用。
  api: async (method: string, url: string, body?: any): Promise<any> => {
    const raw = await invoke<string>("mihomo_api", { method, url, body: body ?? null });
    try {
      return JSON.parse(raw);
    } catch {
      return raw;
    }
  },
  version: () => mihomoApi.api("GET", "/version"),
  proxies: () => mihomoApi.api("GET", "/proxies"),
  groups: () => mihomoApi.api("GET", "/providers/proxies"),
  rules: () => mihomoApi.api("GET", "/rules"),
  proxyProviders: () => mihomoApi.api("GET", "/providers/proxies"),
  ruleProviders: () => mihomoApi.api("GET", "/providers/rules"),
  changeProxy: (group: string, name: string) =>
    mihomoApi.api("PUT", `/proxies/${encodeURIComponent(group)}`, {
      name,
    }),
  proxyDelay: (name: string, url: string, timeout: number) =>
    mihomoApi.api(
      "GET",
      `/proxies/${encodeURIComponent(name)}/delay?url=${encodeURIComponent(
        url
      )}&timeout=${timeout}`
    ),
  setMode: (mode: string) =>
    mihomoApi.api("PATCH", "/configs", { mode }),
  selectProxy: (name: string) => invoke("mihomo_select_proxy", { name }),
  /** 保存整份二级代理列表 + 当前启用项，并热重载 */
  saveSecondaryProxies: (items: SecondaryProxy[], activeId: string | null) =>
    invoke("mihomo_save_secondary_proxies", { items, activeId }),
  setTun: (enable: boolean) => invoke("mihomo_set_tun", { enable }),
  closeConnection: (id: string) =>
    mihomoApi.api("DELETE", `/connections/${encodeURIComponent(id)}`),
  getConnections: () => mihomoApi.api("GET", "/connections"),
  getMemory: () => mihomoApi.api("GET", "/memory"),
  getLogs: () => invoke<string[]>("mihomo_get_logs"),

  // ---- sysproxy ----
  setSysProxy: (enable: boolean) => invoke("mihomo_set_sys_proxy", { enable }),
  getSysProxy: () => invoke<any>("mihomo_get_sys_proxy"),

  // ---- upgrade ----
  upgrade: () => invoke<string>("mihomo_upgrade"),
  upgradeCore: () => invoke<string>("mihomo_upgrade"),
  upgradeGeo: () => invoke<string>("mihomo_upgrade_geo"),
  upgradeUi: () => invoke<any>("mihomo_upgrade_ui"),
  openSubstore: () => invoke<any>("mihomo_open_substore"),

  // ---- 系统集成 ----
  setupFirewall: () => invoke<string>("mihomo_setup_firewall"),
  openUwpTool: () => invoke<string>("mihomo_open_uwp_tool"),
  getOverrideExecLog: (id: string) =>
    invoke<string[]>("mihomo_get_override_exec_log", { id }),

  // ---- 内核版本管理（内核随程序预置，不提供联网安装） ----
  coreVariants: () => invoke<any>("mihomo_core_variants"),
  downloadUi: () => invoke<string>("mihomo_download_ui"),

  // ---- 网络信息 ----
  fetchIpInfo: (url: string, useProxy = true, timeoutMs = 10000) =>
    invoke<any>("mihomo_fetch_ip_info", { url, useProxy, timeoutMs }),
  measureLatency: (url: string, useProxy = true, timeoutMs = 5000) =>
    invoke<number | null>("mihomo_measure_latency", { url, useProxy, timeoutMs }),
  getInterfaces: (force = false) => invoke<any>("mihomo_get_interfaces", { force }),

  // ---- 备份恢复 ----
  exportLocalBackup: (path: string) =>
    invoke<string>("mihomo_export_local_backup", { path }),
  importLocalBackup: (path: string) =>
    invoke<string>("mihomo_import_local_backup", { path }),
  webdavBackup: () => invoke<string>("mihomo_webdav_backup"),
  webdavList: () => invoke<any>("mihomo_webdav_list"),
  webdavRestore: (name: string) =>
    invoke<string>("mihomo_webdav_restore", { name }),
  webdavDelete: (name: string) => invoke("mihomo_webdav_delete", { name }),

  // ---- Sub-Store ----
  substoreDownload: () => invoke<string>("mihomo_substore_download"),
  substoreStart: () => invoke<any>("mihomo_substore_start"),
  substoreStop: () => invoke("mihomo_substore_stop"),
  substoreStatus: () => invoke<any>("mihomo_substore_status"),
  substoreSubs: () => invoke<any>("mihomo_substore_subs"),
  substoreCollections: () => invoke<any>("mihomo_substore_collections"),

  // ---- 杂项 ----
  checkAdmin: () => invoke<boolean>("mihomo_check_admin"),
  restartAsAdmin: () => invoke("mihomo_restart_as_admin"),
  checkTunPermissions: () => invoke<any>("mihomo_check_tun_permissions"),
  hotReload: () => invoke("mihomo_hot_reload"),
  copyEnv: (kind: "cmd" | "powershell" | "bash") =>
    invoke<string>("mihomo_copy_env", { kind }),
  clearLogs: () => invoke("mihomo_clear_logs"),
  openPath: (kind: "data" | "core" | "log") =>
    invoke("mihomo_open_path", { kind }),
  cleanupLogs: () => invoke<number>("mihomo_cleanup_logs"),

  // ---- 内核 API 补充 ----
  updateProxyProvider: (name: string) =>
    mihomoApi.api("PUT", `/providers/proxies/${encodeURIComponent(name)}`),
  healthCheckProvider: (name: string) =>
    mihomoApi.api(
      "GET",
      `/providers/proxies/${encodeURIComponent(name)}/healthcheck`
    ),
  updateRuleProvider: (name: string) =>
    mihomoApi.api("PUT", `/providers/rules/${encodeURIComponent(name)}`),
  flushFakeIp: () => mihomoApi.api("POST", "/cache/fakeip/flush"),
  groupDelay: (group: string, url: string, timeout: number) =>
    mihomoApi.api(
      "GET",
      `/group/${encodeURIComponent(group)}/delay?url=${encodeURIComponent(
        url
      )}&timeout=${timeout}`
    ),
  unfixedProxy: (group: string) =>
    mihomoApi.api("DELETE", `/proxies/${encodeURIComponent(group)}`),
  dnsQuery: (name: string, type = "A") =>
    mihomoApi.api(
      "GET",
      `/dns/query?name=${encodeURIComponent(name)}&type=${type}`
    ),
  restartCoreApi: () => mihomoApi.api("POST", "/restart"),
  upgradeCoreApi: () => mihomoApi.api("POST", "/upgrade"),
  upgradeUiApi: () => mihomoApi.api("POST", "/upgrade/ui"),
  upgradeGeoApi: () => mihomoApi.api("POST", "/configs/geo"),

  // ---- 内核 API（后端命令实现，带鉴权与超时，对齐 clash-party mihomoApi.ts）----
  groupDelayCmd: (group: string, url: string, timeout: number) =>
    invoke<any>("mihomo_group_delay", { group, url, timeout }),
  providerHealthcheck: (
    provider: string,
    url: string,
    timeout: number,
    name?: string
  ) =>
    invoke<any>("mihomo_provider_healthcheck", {
      provider,
      name: name ?? null,
      url,
      timeout,
    }),
  updateProxyProviderCmd: (name: string) =>
    invoke("mihomo_update_proxy_provider", { name }),
  updateRuleProviderCmd: (name: string) =>
    invoke("mihomo_update_rule_provider", { name }),
  /** 批量启用/禁用规则：{ "规则内容": true/false } */
  rulesDisable: (rules: Record<string, boolean>) =>
    invoke("mihomo_rules_disable", { rules }),
  smartGroupWeights: (group: string) =>
    invoke<Record<string, number>>("mihomo_smart_group_weights", { group }),
  smartFlushCache: (configName?: string) =>
    invoke("mihomo_smart_flush_cache", { configName: configName ?? null }),
  patchConfig: (patch: any) => invoke("mihomo_patch_config", { patch }),
  hotReloadConfig: () => invoke("mihomo_hot_reload_config"),

  // ---- 规则覆写 / 文件 ----
  getRuleStr: (id: string) => invoke<string>("mihomo_get_rule_str", { id }),
  setRuleStr: (id: string, content: string) =>
    invoke("mihomo_set_rule_str", { id, content }),
  getRuleOverride: (id: string) =>
    invoke<{ prepend: any[]; append: any[]; delete: any[] }>(
      "mihomo_get_rule_override",
      { id }
    ),
  setRuleOverride: (
    id: string,
    data: { prepend: any[]; append: any[]; delete: any[] }
  ) => invoke("mihomo_set_rule_override", { id, data }),
  getFileStr: (path: string) => invoke<string>("mihomo_get_file_str", { path }),
  setFileStr: (path: string, content: string) =>
    invoke("mihomo_set_file_str", { path, content }),
  convertMrsRuleset: (
    ruleType: "domain" | "ipcidr",
    inputFormat: "yaml" | "text",
    inputPath: string,
    outputPath: string
  ) =>
    invoke<string>("mihomo_convert_mrs_ruleset", {
      ruleType,
      inputFormat,
      inputPath,
      outputPath,
    }),
  detachCore: () => invoke("mihomo_detach_core"),
};

export type AppConfig = {
  control_dns: boolean;
  control_sniff: boolean;
  use_nameserver_policy: boolean;
  sys_proxy_enable: boolean;
  auto_start_core: boolean;
  auto_set_proxy: boolean;
  auto_close_proxy: boolean;
  tun_enabled: boolean;
  mixed_port: number;
  controller_port: number;
  secret: string;
  proxy_cols: number;
  proxy_sort_type: string;
  keep_profile_alive: boolean;
  theme: string;
  lang: string;
  ipc_port?: number;
  sys_proxy_bypass: string;
  substore_enabled: boolean;
  webdav_url: string;
  webdav_user: string;
  webdav_pass: string;
  webdav_auto_backup: boolean;
  current_profile: string;
  /** 独立工作目录：每个订阅使用单独的内核工作目录 */
  diff_work_dir?: boolean;
  core_path?: string | null;
  /** 一级代理（代理页选中的节点/组名，作为二级代理的 dialer-proxy） */
  default_proxy?: string | null;
  /** 二级代理列表（家庭 socks5），可多个 */
  secondary_proxies?: SecondaryProxy[];
  /** 当前启用的二级代理 id */
  secondary_active_id?: string | null;
  /** 内核 CPU 优先级（Windows PRIORITY_CLASS 名） */
  cpuPriority?: string;
  /** 启动前用 `mihomo -t` 预校验配置 */
  testProfileOnStart?: boolean;
  /** 其余扁平化透传字段 */
  [key: string]: any;
};

export type SecondaryProxy = {
  id: string;
  name: string;
  host: string;
  port: number;
  username?: string | null;
  password?: string | null;
};
