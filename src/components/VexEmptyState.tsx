import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import VexGlowAvatar from "./VexGlowAvatar";
import { SharedButton } from "./shared/Button";

/** 空态里的操作按钮：主操作（实心）+ 可选次操作（描边）。 */
export interface VexEmptyAction {
  label: string;
  onClick: () => void;
}

/**
 * Kira 风格的空态：头像 + 一句人设口吻的说明，替代冷冰冰的「暂无数据」。
 * 用于导图空文档 / 无订阅 / 无历史 / 无结果 / 空曲库等空状态。
 *
 * 全站空态统一走这里：外观（头像 + 标题 + 说明 + 口头语 + 操作）只有一份，
 * 各模块只负责传文案与动作，避免每个面板各造一套「暂无数据」。
 */
export default function VexEmptyState({
  title,
  desc,
  tick,
  tickColor = "text-[var(--module-accent)]",
  avatarSize = 48,
  className = "",
  action,
  secondaryAction,
}: {
  title?: string;
  /** 说明：允许 JSX（例如把「拖入此处」染成主题色）。 */
  desc?: ReactNode;
  /** 一句 Kira 口头语，点缀在空态下方，让人味更足。 */
  tick?: string;
  tickColor?: string;
  avatarSize?: number;
  className?: string;
  /** 主操作（如「导入文件夹」）：空态里唯一被强调的出口。 */
  action?: VexEmptyAction;
  /** 次操作（如「新建子分组」）：与主操作并排，弱一级。 */
  secondaryAction?: VexEmptyAction;
}) {
  const { t } = useTranslation();
  return (
    <div className={`flex flex-col items-center justify-center gap-3 py-14 text-center ${className}`}>
      <VexGlowAvatar size={avatarSize} />
      <div>
        <p className="text-body text-slate-400">{title ?? t("vex.defaultTitle")}</p>
        {desc !== undefined && <p className="mt-1 text-caption text-slate-600">{desc ?? t("vex.defaultDesc")}</p>}
      </div>
      {tick && (
        <p className={`text-tiny italic opacity-80 ${tickColor}`}>— {tick}</p>
      )}
      {(action || secondaryAction) && (
        <div className="mt-1 flex flex-wrap items-center justify-center gap-2">
          {action && (
            <SharedButton variant="primary" onClick={action.onClick}>
              {action.label}
            </SharedButton>
          )}
          {secondaryAction && (
            <SharedButton variant="secondary" onClick={secondaryAction.onClick}>
              {secondaryAction.label}
            </SharedButton>
          )}
        </div>
      )}
    </div>
  );
}