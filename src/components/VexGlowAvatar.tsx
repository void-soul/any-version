import VexAvatar from "./VexAvatar";
import { VEX_AVATAR, VEX_CYBER_ACCENT } from "../utils/brand";

/**
 * 带「坏灯管」辉光的 Kira 头像——全 App 统一入口。
 *
 * 结构：底层一张同图副本做「光晕层」，上层是真正的人物。
 * - 光晕层用 drop-shadow 发光：投影按元素渲染结果的 alpha 轮廓计算，所以光沿
 *   裁剪切边往外渗，而不是靠 box-shadow 画出的硬圆环。
 * - 光晕层必须与前台用**完全相同的裁剪**（同尺寸、同位置、同圆角类）：logo.png
 *   是不透明方图（不是抠好的透明人物 PNG），光晕层若不加圆角，整张方形副本
 *   （opacity 0.8）会在前台圆形裁剪之外的四角露成一块暗色方形底，并跟着闪烁
 *   动画一亮一灭 —— 看起来像「背景在闪、头像忽圆忽方」。
 * - 加上同样的圆角后，光晕层圆内像素被上层人物完全覆盖，只有 drop-shadow 的
 *   外圈辉光可见；闪烁动画（.vex-flicker）只作用在光晕层的 opacity 上，人物
 *   本身不动。
 * - 节奏见 App.css 的 vex-flicker：快速闪两次 → 长亮一次 → 快速闪三次 →
 *   常亮（steps(1,end) 硬跳变；系统开启「减少动态效果」时自动退回常亮）。
 * - 人物默认完全静止（不呼吸缩放）；要恢复动感传 breathe。
 */
export default function VexGlowAvatar({
  size = 32,
  color,
  flicker = true,
  round = true,
  breathe = false,
  className = "",
  avatarClassName = "",
  title,
}: {
  /** 头像像素尺寸（方形边长 / 圆形直径） */
  size?: number;
  /** 辉光颜色，默认跟随当前模块主题色 --module-accent */
  color?: string;
  /** 是否启用坏灯管闪烁；false 时辉光常亮 */
  flicker?: boolean;
  /** 是否把人物裁剪为圆形（光晕始终沿人物轮廓，不受此项影响） */
  round?: boolean;
  /** true = 恢复人物呼吸缩放/悬停动感；默认人物大小固定不动 */
  breathe?: boolean;
  /** 加在外层定位容器上的类 */
  className?: string;
  /** 加在人物 <img> 上的类（如 vex-neon-breathe） */
  avatarClassName?: string;
  title?: string;
}) {
  const accent = color ?? `var(--module-accent, ${VEX_CYBER_ACCENT})`;
  const haloCls = flicker ? "vex-flicker" : "";
  // 与 <VexAvatar> 保持同一套裁剪：光晕层若不同裁剪，方形副本会在四角露出（见文件头注释）
  const roundCls = round ? "rounded-full" : "rounded-ctl";
  const glow = `drop-shadow(0 0 ${Math.max(3, Math.round(size * 0.1))}px color-mix(in srgb, ${accent} 90%, transparent)) drop-shadow(0 0 ${Math.max(8, Math.round(size * 0.28))}px color-mix(in srgb, ${accent} 45%, transparent))`;

  return (
    <span className={`relative inline-flex flex-shrink-0 ${className}`}>
      {/* 光晕层：同图副本 + 沿轮廓 drop-shadow，闪烁只改这层的透明度 */}
      <img
        aria-hidden
        alt=""
        src={VEX_AVATAR}
        draggable={false}
        className={`pointer-events-none absolute inset-0 h-full w-full object-cover select-none ${roundCls} ${
          breathe ? "vex-breathe" : ""
        } ${haloCls}`}
        style={{ filter: glow, opacity: 0.8 }}
      />
      <VexAvatar
        size={size}
        round={round}
        title={title}
        still={!breathe}
        className={`relative ${avatarClassName}`}
      />
    </span>
  );
}
