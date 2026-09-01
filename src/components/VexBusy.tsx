import VexAvatar from "./VexAvatar";
import { VEX_CYBER_ACCENT, VEX_CYBER_CYAN } from "../utils/brand";
import { useTranslation } from "react-i18next";

/**
 * Kira 「忙碌小助手」：替代干巴巴的转圈，让 Kira 的面容 + 霓虹加载条 + 一句伴随语
 * 陪用户等待（导入 / 扫描 / AI 生成 / 翻译…）。
 */
export default function VexBusy({
  text,
  avatarSize = 34,
  barColor,
}: {
  /** 伴随语，可为 JSX（如高亮进行中的项名） */
  text?: React.ReactNode;
  avatarSize?: number;
  /** 加载条主色，默认跟随当前主题色（--module-accent）到青的渐变 */
  barColor?: string;
}) {
  const { t } = useTranslation();
  const resolvedText = text ?? t("vexbusy.defaultText");
  const accent = `var(--module-accent, ${VEX_CYBER_ACCENT})`;
  return (
    <div className="flex items-center gap-3">
      <div className="relative flex-shrink-0">
        <VexAvatar size={avatarSize} />
        <span
          className="absolute inset-0 -z-10 animate-ping rounded-full"
          style={{ boxShadow: `0 0 24px ${barColor ?? accent}` }}
        />
      </div>
      <div className="min-w-0 flex-1">
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
          <div
            className="h-full rounded-full animate-[vexbusybar_1.3s_ease-in-out_infinite]"
            style={{
              background: `linear-gradient(90deg, ${barColor ?? accent}, ${VEX_CYBER_CYAN})`,
            }}
          />
        </div>
        <p className="mt-1.5 truncate text-[10px] text-slate-400">{resolvedText}</p>
      </div>
    </div>
  );
}