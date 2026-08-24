// 读取当前激活模块的主题色（--module-accent）。
// App 在内容区包裹层通过内联样式注入该 CSS 变量，任务画布/节点等模块内
// 组件可用它做“默认节点颜色 = 模块主题色”的联动。读取失败时回退琥珀色
// （任务模块默认主题色），避免返回空串导致节点变色异常。
let cached: { value: string; at: number } | null = null;

export function moduleAccent(): string {
  const now = Date.now();
  if (cached && now - cached.at < 5000) return cached.value;
  let value = "";
  if (typeof document !== "undefined") {
    const el = document.getElementById("app-content");
    if (el) {
      value = getComputedStyle(el).getPropertyValue("--module-accent").trim();
    }
  }
  if (!/^#[0-9a-f]{6}$/i.test(value)) value = "#f59e0b";
  cached = { value, at: now };
  return value;
}

/** 把模块主题色渲染为带透明度的颜色（如边框、浅底），hex → rgba。 */
export function accentWithAlpha(alpha: number): string {
  const hex = moduleAccent();
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
