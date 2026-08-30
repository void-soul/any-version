import { useMemo } from "react";
import { VEX_AVATAR, VEX_CYBER_ACCENT } from "../utils/brand";

/**
 * Vex 头像——全 App 统一入口。
 * 资源固定从 public/logo.png（/logo.png）读取：想换头像，替换该文件即可，所有引用同步更新。
 * 自带「生命力」动效：常驻呼吸缩放 + 悬停俏皮歪头；默认辉光跟随当前主题色
 * （--module-accent，未注入时回退默认签名色）。
 */
export default function VexAvatar({
  size = 32,
  round = true,
  className = "",
  title,
  glow,
}: {
  /** 像素尺寸（方形边长；也是圆形头像直径） */
  size?: number;
  /** 是否裁剪为圆形 */
  round?: boolean;
  className?: string;
  title?: string;
  /** 外圈辉光色（默认跟随当前主题色） */
  glow?: string;
}) {
  const color = glow ?? `var(--module-accent, ${VEX_CYBER_ACCENT})`;
  const glowCss = useMemo(
    () => `0 0 ${Math.round(size * 0.4)}px ${color}, 0 0 ${Math.round(size * 1.2)}px ${color}40`,
    [color, size],
  );

  return (
    <img
      src={VEX_AVATAR}
      className={`object-cover select-none vex-breathe vex-hover ${round ? "rounded-full" : "rounded-lg"} ${className}`}
      style={{
        width: size,
        height: size,
        boxShadow: glowCss,
      }}
      alt="Kira"
      title={title ?? "Kira"}
      draggable={false}
    />
  );
}