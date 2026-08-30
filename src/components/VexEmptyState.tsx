import VexAvatar from "./VexAvatar";

/**
 * vex 风格的空态：头像 + 一句人设口吻的说明，替代冷冰冰的「暂无数据」。
 * 用于导图空文档 / 无订阅 / 无历史 / 无结果等空状态。
 */
export default function VexEmptyState({
  title = "这里还空着～",
  desc,
  avatarSize = 48,
  className = "",
}: {
  title?: string;
  desc?: string;
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
    </div>
  );
}