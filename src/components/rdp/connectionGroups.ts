import { parseGroupPath } from "./sessionTabs";

export interface GroupableConnection {
  id: string;
  name: string;
  group: string;
}

export interface ConnectionGroup<T extends GroupableConnection> {
  /** 稳定 key，可直接用于折叠状态；未分组为空字符串。 */
  key: string;
  label: string;
  isUngrouped: boolean;
  items: T[];
}

/**
 * 扁平连接聚合成分组：按名称排序，未分组固定排最后。
 * 组内同样按名称排序，保证每次渲染顺序稳定（不随保存顺序抖动）。
 */
export function buildGroups<T extends GroupableConnection>(list: T[]): ConnectionGroup<T>[] {
  const map = new Map<string, ConnectionGroup<T>>();
  for (const item of list) {
    const path = parseGroupPath(item.group);
    const existing = map.get(path.label);
    if (existing) {
      existing.items.push(item);
    } else {
      map.set(path.label, {
        key: path.label,
        label: path.label,
        isUngrouped: path.isUngrouped,
        items: [item],
      });
    }
  }
  const groups = [...map.values()];
  for (const g of groups) g.items.sort((a, b) => a.name.localeCompare(b.name));
  return groups.sort((a, b) => {
    if (a.isUngrouped !== b.isUngrouped) return a.isUngrouped ? 1 : -1;
    return a.label.localeCompare(b.label);
  });
}

/** `group === null` 表示不筛选；空字符串精确匹配未分组。 */
export function filterConnections<T extends GroupableConnection>(list: T[], group: string | null): T[] {
  if (group === null) return list;
  return list.filter((item) => parseGroupPath(item.group).label === group);
}

/** 所有已存在的分组标签（不含未分组），供编辑表单做下拉建议。 */
export function groupOptions<T extends GroupableConnection>(list: T[]): string[] {
  const set = new Set<string>();
  for (const item of list) {
    const { label } = parseGroupPath(item.group);
    if (label) set.add(label);
  }
  return [...set].sort((a, b) => a.localeCompare(b));
}
