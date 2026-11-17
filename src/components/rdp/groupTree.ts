/**
 * 远程连接的「文件夹」模型。
 *
 * 分组就是文件夹，路径存在连接自己的 `group` 字段里（`prod/web` 表示放在
 * 根下 `prod/web` 这个文件夹）。这样不需要给连接维护额外关联，拖拽只是改路径。
 * 空文件夹靠 `extraFolders`（后端单独存）参与建模，否则没有连接的文件夹会消失。
 */

export interface TreeConnection {
  id: string;
  name: string;
  group: string;
}

export interface FolderNode<T extends TreeConnection> {
  /** 完整路径，根为空串 */
  path: string;
  /** 最后一段名字 */
  name: string;
  depth: number;
  folders: FolderNode<T>[];
  items: T[];
}

/** 统一分隔符（Windows 习惯的反斜杠也接受）、去掉首尾与重复斜杠。 */
export function normalizePath(raw: string): string {
  const normalized = raw.replace(/\\/g, "/");
  const parts = normalized
    .split("/")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  if (parts.length === 1 && parts[0].toLowerCase() === "root") return "";
  return parts.join("/");
}

export function splitPath(path: string): string[] {
  const normalized = normalizePath(path);
  return normalized ? normalized.split("/") : [];
}

/** 包含自身在内的整棵子树路径。 */
export function descendants(path: string, all: string[]): string[] {
  const base = normalizePath(path);
  const prefix = base ? `${base}/` : "";
  return all.filter((p) => {
    const current = normalizePath(p);
    if (!base) return true;
    return current === base || current.startsWith(prefix);
  });
}

/**
 * 把 `path` 从 `from` 子树搬到 `to` 子树下。
 * 只承认真正的层级关系（`ab` 不属于 `a`），避免误改同前缀的兄弟文件夹。
 */
export function rebasePath(path: string, from: string, to: string): string {
  const current = normalizePath(path);
  const src = normalizePath(from);
  const dst = normalizePath(to);
  if (!src) return current;
  if (current === src) return dst;
  const prefix = `${src}/`;
  if (!current.startsWith(prefix)) return current;
  const rest = current.slice(prefix.length);
  return dst ? `${dst}/${rest}` : rest;
}

const emptyNode = <T extends TreeConnection>(path: string, name: string, depth: number): FolderNode<T> => ({
  path,
  name,
  depth,
  folders: [],
  items: [],
});

/** 建树：先登记所有文件夹（连接路径 + 空文件夹），再挂连接。 */
export function buildTree<T extends TreeConnection>(items: T[], extraFolders: string[]): FolderNode<T> {
  const root = emptyNode<T>("", "", -1);
  const index = new Map<string, FolderNode<T>>([["", root]]);

  const ensure = (path: string): FolderNode<T> => {
    const existing = index.get(path);
    if (existing) return existing;
    const parts = splitPath(path);
    const parentPath = parts.slice(0, -1).join("/");
    const parent = ensure(parentPath);
    const node = emptyNode<T>(path, parts[parts.length - 1] ?? "", parent.depth + 1);
    parent.folders.push(node);
    index.set(path, node);
    return node;
  };

  for (const folder of extraFolders) {
    const path = normalizePath(folder);
    if (path) ensure(path);
  }
  for (const item of items) {
    ensure(normalizePath(item.group)).items.push(item);
  }

  // 文件夹与连接都按名称排序，保证顺序稳定、不随保存顺序抖动
  const sortNode = (node: FolderNode<T>) => {
    node.folders.sort((a, b) => a.name.localeCompare(b.name));
    node.items.sort((a, b) => a.name.localeCompare(b.name));
    node.folders.forEach(sortNode);
  };
  sortNode(root);
  return root;
}

/** 所有已存在的文件夹路径（含空文件夹），用于重命名时批量改写。 */
export function collectFolders<T extends TreeConnection>(root: FolderNode<T>): string[] {
  const out: string[] = [];
  const walk = (node: FolderNode<T>) => {
    for (const child of node.folders) {
      out.push(child.path);
      walk(child);
    }
  };
  walk(root);
  return out;
}
