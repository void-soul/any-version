import { useTranslation } from "react-i18next";
import { EDGE_STYLES, type EdgeStyle } from "../../utils/graphEdgeStyle";

/**
 * 连线样式选择器：ER 设计 / 思维导图 / JSON 图共用的那一个下拉。
 * 只出取值与文案，配色由调用方的 className 决定（三处工具栏风格各不相同）。
 */
export default function EdgeStyleSelect({
  value,
  onChange,
  className = "",
}: {
  value: EdgeStyle;
  onChange: (next: EdgeStyle) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value as EdgeStyle)}
      title={t("common.edgeStyle")}
      className={className}
    >
      {EDGE_STYLES.map((style) => (
        <option key={style} value={style}>
          {t(`common.edge_${style}`)}
        </option>
      ))}
    </select>
  );
}
