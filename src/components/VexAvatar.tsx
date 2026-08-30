import { useMemo } from "react";
import { VEX_AVATAR, VEX_CYBER_ACCENT } from "../utils/brand";

/**
 * Vex 头像（二次元元气少女）——全 App 统一入口。
 * 资源固定从 public/logo.png（/logo.png）读取：想换头像，替换该文件即可，所有引用同步更新。
 * 自带「生命力」动效：常驻呼吸缩放 + 悬停俏皮歪头；默认带赛博霓虹辉光。
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
  /** 外圈辉光色（赛博主色默认） */
  glow?: string;
}) {
  const color = glow ?? VEX_CYBER_ACCENT;
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
      alt="vex"
      title={title ?? "vex"}
      draggable={false}
    />
  );
}