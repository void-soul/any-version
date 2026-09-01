/** 节点配色预设：渐变细条取色器的色标序列（两端到中间线性过渡）。 */
export const COLOR_STOPS = [
  "#f8fafc", "#22d3ee", "#34d399", "#fbbf24", "#60a5fa",
  "#fb7185", "#a78bfa", "#f97316", "#f59e0b", "#94a3b8",
];

/** 颜色 → 色相百分比（0~100），用于渐变细条取色器的选中标记定位（自定义色也准）。 */
export function huePercent(hexColor: string): number {
  const h = hexColor.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16) / 255;
  const g = parseInt(h.slice(2, 4), 16) / 255;
  const b = parseInt(h.slice(4, 6), 16) / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let hue = 0;
  if (max !== min) {
    if (max === r) hue = 60 * (((g - b) / (max - min)) % 6);
    else if (max === g) hue = 60 * ((b - r) / (max - min) + 2);
    else hue = 60 * ((r - g) / (max - min) + 4);
  }
  if (hue < 0) hue += 360;
  return Math.min(97, Math.max(3, (hue / 360) * 100));
}

/** 渐变细条取色器：点击任意位置取色（含相邻色标间插值），
 *  选中标记按色相定位。供画布内 DetailModal 与速记悬浮窗共用。 */
export function ColorGradientBar({ color, onChange }: { color: string; onChange: (color: string) => void }) {
  return (
    <div
      className="relative h-3.5 flex-1 cursor-pointer rounded-full border border-white/20"
      style={{ background: `linear-gradient(90deg, ${COLOR_STOPS.join(", ")})` }}
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        const t = Math.max(0, Math.min(0.9999, (e.clientX - r.left) / r.width));
        const i = Math.floor(t * (COLOR_STOPS.length - 1));
        const f = t * (COLOR_STOPS.length - 1) - i;
        const a = COLOR_STOPS[i], b = COLOR_STOPS[i + 1];
        const hex = (n: number) => Math.round(n).toString(16).padStart(2, "0");
        const pa = [1, 3, 5].map((k) => parseInt(a.slice(k, k + 2), 16));
        const pb = [1, 3, 5].map((k) => parseInt(b.slice(k, k + 2), 16));
        onChange(`#${pa.map((x, k) => hex(x + (pb[k] - x) * f)).join("")}`);
      }}
    >
      {/* 选中标记：按色相定位 */}
      <span
        className="pointer-events-none absolute top-1/2 h-4 w-1 -translate-y-1/2 rounded-full bg-white shadow ring-1 ring-black/40"
        style={{ left: `${/^#[0-9a-fA-F]{6}$/.test(color) ? huePercent(color) : 0}%` }}
      />
    </div>
  );
}
