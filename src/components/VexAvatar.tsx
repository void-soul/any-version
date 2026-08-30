import { useMemo } from "react";
import { VEX_AVATAR } from "../utils/brand";

/**
 * Vex 头像（二次元元气少女）——全 App 统一入口。
 * 资源固定从 public/logo.png（/logo.png）读取：想换头像，替换该文件即可，所有引用同步更新。
 * 附带轻微的「生命力」动效：常驻呼吸缩放 + 悬停时俏皮眨一下（可关）。
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
  /** 外圈辉光色（模块主题色等），不传则用默认紫 */
  glow?: string;
}) {
  const glowCss = useMemo(
    () =>
      glow
        ? `0 0 ${Math.round(size * 0.4)}px ${glow}, 0 0 ${Math.round(size * 1.2)}px ${glow}33`
        : undefined,
    [glow, size],
  );

  return (
    <img
      src={VEX_AVATAR}
      className={`object-cover select-none ${round ? "rounded-full" : "rounded-lg"} ${className}`}
      style={{
        width: size,
        height: size,
        boxShadow: glow ? glowCss : undefined,
      }}
      alt="vex"
      title={title ?? "vex"}
      draggable={false}
    />
  );
}