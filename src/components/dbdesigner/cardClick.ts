// 卡片点击分流：**点表头 = 只高亮，点字段区域 = 打开底部编辑面板**。
//
// 为什么抽成模块而不是留在面板里：这段是纯分支逻辑（读一个 data 属性 → 决定行为），
// 却要嵌在 2600 行的面板组件里才能测 —— 面板带 i18n / Tauri / React Flow 依赖，
// 单测根本拖不进来。抽出来后规则可以直接被断言（见 __tests__/cardClick.test.ts）。
//
// 靠 `data-dbd-role` 而不是把回调塞进 node data：`rfNodes` 的 useMemo 依赖 data，
// 每次渲染新建函数会让整张图重算，而这里只需要读一个字符串。

/** 卡片上标记点击区域的属性名（与 DesignNodeCard 里写的一致） */
export const CARD_ROLE_ATTR = "data-dbd-role";

/** 表头：点它 = 选中并高亮，不开面板 */
export const ROLE_HEADER = "table-header";
/** 单个字段行：点它 = 开面板，不高亮 */
export const ROLE_FIELD_ROW = "field-row";
/** 字段区容器（含「还没有字段」时的空白）：同样算「我要看字段」 */
export const ROLE_FIELD_AREA = "field-area";

export type CardClickAction =
  /** 打开底部面板编辑这张表 */
  | { kind: "openDock" }
  /** 只高亮（含它的一跳关联），不开面板 */
  | { kind: "highlight" };

/**
 * 从事件目标取出最近的 `data-dbd-role` 值。
 *
 * 用 `closest` 而不是 `dataset` 直接读：字段行内部还有 `<span>`、锚点等子元素，
 * 点在它们上面也要算「点字段」。target 不是 Element 时（理论上不会）返回 null。
 */
export function clickRoleOf(target: EventTarget | null): string | null {
  const el = target as Element | null;
  if (!el || typeof el.closest !== "function") return null;
  const hit = el.closest(`[${CARD_ROLE_ATTR}]`);
  return hit ? hit.getAttribute(CARD_ROLE_ATTR) : null;
}

/**
 * 角色 → 行为。
 *
 * **未知角色按「高亮」处理**（fail-safe 到旧行为）：新增标记时忘了在这里加规则，
 * 表现是「点了没开面板」，而不是「点了什么反应都没有」—— 前者用户知道点错了地方。
 */
export function cardClickAction(role: string | null | undefined): CardClickAction {
  return role === ROLE_FIELD_ROW || role === ROLE_FIELD_AREA
    ? { kind: "openDock" }
    : { kind: "highlight" };
}
