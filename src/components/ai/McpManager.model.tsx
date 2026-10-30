
/** 后端 resolve_mcp_source 的草稿（camelCase） */
export interface McpSourceDraft {
  /** npx | repo | local | http */
  kind: string;
  name: string;
  transport: string;
  command: string;
  args: string[];
  url: string;
  localPath: string;
  /** 探测说明（探测不到时提示用户自己填命令） */
  note: string;
}


/** 托管安装来源：从 GitHub Release 下载发行包并解压到本应用数据目录 */
export interface McpManagedSource {
  /** owner/repo */
  repo: string;
  /** tag 模板，如 v{version} */
  tagTemplate: string;
  /** 资产名前缀模板，如 name-{version} */
  assetPrefix: string;
  /** 资产名后缀，如 .zip */
  assetSuffix: string;
  /** 解压后要进入的子目录模板（空 = 自动剥掉唯一顶层目录） */
  extractSubdir: string;
  /** 可执行文件相对安装根的路径，如 bin/xxx.bat */
  executable: string;
}


export interface McpManagedStatus {
  installedVersions: string[];
  /** 当前 command 指向的文件是否存在（决定能否装到项目） */
  executableExists: boolean;
  command: string;
}


export interface McpServer {
  id: string;
  name: string;
  transport: string; // stdio | http | sse
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  url: string;
  headers: Record<string, string>;
  enabled: boolean;
  enabledTools: string[];
  description: string | null;
  installMethod: string;
  /** 有值时由本应用下载安装（command 指向解压出来的可执行文件） */
  managed?: McpManagedSource | null;
}


/** 项目里已装的 MCP（后端 list_project_mcps） */
export interface ProjectMcp {
  /** 在 `<项目>/.mcp.json` 里的键名 */
  name: string;
  transport: string;
  command: string;
  args: string[];
  url: string;
  /** 仓库里有同名定义 = 受管；否则是用户手写的「项目自带」 */
  managed: boolean;
}


export interface DiscoveredMcp {
  toolId: string;
  name: string;
  transport: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  url: string;
  headers: Record<string, string>;
  alreadyManaged: boolean;
}


/** MCP 服务器预设：只用于预填表单，不直接落库（命令/地址用户通常还要改）。 */
export interface McpPreset {
  id: string;
  name: string;
  description: string;
  transport: string;
  command: string;
  args: string[];
  url: string;
  /** 依赖的本地服务 id（如 wigolo）；空表示无依赖 */
  serviceId: string;
  /** 预设自带托管来源：添加后可直接点「安装」 */
  managed?: McpManagedSource | null;
}


// 将 "KEY=VALUE" 多行文本解析为对象
export function parseKV(text: string): Record<string, string> {
  const obj: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const idx = t.indexOf("=");
    if (idx <= 0) continue;
    const k = t.slice(0, idx).trim();
    const v = t.slice(idx + 1).trim();
    if (k) obj[k] = v;
  }
  return obj;
}


// 将对象序列化为 "KEY=VALUE" 多行文本
export function serializeKV(obj: Record<string, string>): string {
  return Object.entries(obj).map(([k, v]) => `${k}=${v}`).join("\n");
}


export const TRANSPORT_KEY: Record<string, string> = {
  stdio: "transportStdio",
  http: "http",
  sse: "sse",
};
