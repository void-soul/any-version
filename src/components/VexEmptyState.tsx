import VexAvatar from "./VexAvatar";

/**
 * Kira 风格的空态：头像 + 一句人设口吻的说明，替代冷冰冰的「暂无数据」。
 * 用于导图空文档 / 无订阅 / 无历史 / 无结果等空状态。
 */
export default function VexEmptyState({
  title = "这里暂时还空着",
  desc = "没事，想到什么再回来就行。",
  tick,
  tickColor = "text-[var(--module-accent)]",
  avatarSize = 48,
  className = "",
}: {
  title?: string;
  desc?: string;
  /** 一句 Kira 口头语，点缀在空态下方，让人味更足。 */
  tick?: string;
  tickColor?: string;
  avatarSize?: number;
  className?: string;
}) {
  return (
    <div className={`flex flex-col items-center justify-center gap-3 py-14 text-center ${className}`}>
      <VexAvatar size={avatarSize} />
      <div>
        <p className="text-xs text-slate-400">{title}</p>
        {desc && <p className="mt-1 text-[11px] text-slate-600">{desc}</p>}
      </div>
      {tick && (
        <p className={`text-[10px] italic opacity-80 ${tickColor}`}>— {tick}</p>
      )}
    </div>
  );
}