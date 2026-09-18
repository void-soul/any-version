import { VEX_AVATAR } from "../utils/brand";

/**
 * Vex 头像贴图——纯头像 <img>，只负责「把人物画出来」。
 * 资源固定从 public/logo.png（/logo.png）读取：想换头像，替换该文件即可，所有引用同步更新。
 *
 * 注意：本组件不带任何光晕/动效。辉光（沿人物轮廓的 drop-shadow）与坏灯管
 * 闪烁统一由 <VexGlowAvatar> 负责——box-shadow 会跟随圆形边框形成一个
 * 圆形光圈，与「光贴着人物」的要求冲突，所以不要在这里加 box-shadow。
 */
export default function VexAvatar({
  size = 32,
  round = true,
  className = "",
  title,
  still = false,
}: {
  /** 像素尺寸（方形边长；也是圆形头像直径） */
  size?: number;
  /** 是否裁剪为圆形 */
  round?: boolean;
  className?: string;
  title?: string;
  /** true = 人物完全静止（不带呼吸缩放/悬停歪头）；VexGlowAvatar 场景默认静止 */
  still?: boolean;
}) {
  return (
    <img
      src={VEX_AVATAR}
      className={`object-cover select-none ${still ? "" : "vex-breathe vex-hover"} ${round ? "rounded-full" : "rounded-lg"} ${className}`}
      style={{ width: size, height: size }}
      alt="Kira"
      title={title ?? "Kira"}
      draggable={false}
    />
  );
}
