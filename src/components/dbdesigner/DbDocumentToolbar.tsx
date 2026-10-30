import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import EdgeStyleSelect from "../shared/EdgeStyleSelect";
import { DIALECTS, FIELD_ROW_MODES, type FieldRowMode, type DbDesignDocument, type Dialect } from "./types";
import { FIELD_ROWS_KEY } from "./DbDesignerPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface DbDocumentToolbarProps {
  doc: DbDesignDocument;
  patchDoc: (next: DbDesignDocument) => void;
  edgeStyle: "bezier" | "smoothstep" | "straight";
  setEdgeStyle: (next: "bezier" | "smoothstep" | "straight") => void;
  fieldRows: FieldRowMode;
  setFieldRows: Dispatch<SetStateAction<FieldRowMode>>;
  dirty: boolean;
  path: string;
}

/** Pure DbDocumentToolbar view; its owning panel retains side effects and state. */
export function DbDocumentToolbar({ doc, patchDoc, edgeStyle, setEdgeStyle, fieldRows, setFieldRows, dirty, path }: DbDocumentToolbarProps) {
  const { t } = useTranslation();
  return (
    <div className="ml-auto flex items-center gap-2">
      <input
        value={doc.name}
        onChange={(e) => patchDoc({ ...doc, name: e.target.value })}
        className="w-40 rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-100 outline-none focus:border-[var(--module-accent-ring)]"
      />
      <select
        value={doc.dialect}
        onChange={(e) => patchDoc({ ...doc, dialect: e.target.value as Dialect })}
        className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
      >
        {DIALECTS.map((d) => (
          <option key={d} value={d}>{d}</option>
        ))}
      </select>
      {/* 连线样式：只影响观感，不进设计文件；拖线时的实时预览也跟着换 */}
      <EdgeStyleSelect
        value={edgeStyle}
        onChange={setEdgeStyle}
        className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
      />
      {/* 节点卡片里显示哪些字段行：主键+外键（精简）/ 全部字段（展开，视图同时展开 SQL） */}
      <select
        value={fieldRows}
        onChange={(e) => {
          const next = e.target.value as FieldRowMode;
          setFieldRows(next);
          localStorage.setItem(FIELD_ROWS_KEY, next);
        }}
        title={t("dbd.fieldRows")}
        className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
      >
        {FIELD_ROW_MODES.map((mode) => (
          <option key={mode} value={mode}>
            {t(`dbd.fieldRows_${mode}`)}
          </option>
        ))}
      </select>
      <span className="text-micro text-slate-600">
        {dirty ? t("dbd.unsaved") : path ? t("dbd.savedShort") : ""}
      </span>
    </div>
  );
}
