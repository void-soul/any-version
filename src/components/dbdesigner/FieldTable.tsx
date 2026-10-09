// 字段表格：PowerDesigner 的列式表格 + API 模块 EnvModal 的视觉语言。
//
// 放在底部 dock 的「字段」页签里（dock 负责外框 / 页签 / 表名，这里只管表格本体）。
// 为什么是表格而不是卡片：属性有 9 列（名称/类型/长度/P/自增/唯一/可空/默认值/备注），
// 摊成卡片就必然松散。
//
// 照抄 EnvModal 的两处关键做法：
//   1. 键名类输入（字段名）用 defaultValue + onBlur 提交 —— 它同时是 React key 和后端
//      关联引用（dbd_rename_field 要级联改关联与索引），受控会每敲一个字符重建整行。
//   2. 普通值单元格用受控 value + onChange 即时写回；所有单元格共用一个 cellCls，
//      默认值和注释因此外观完全一致（之前一个带底色一个透明，是「外观不一致」的来源）。
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";

import { BASE_TYPES, type DbField, type DbTableBody } from "./types";

/** 单元格内输入框的统一外观（默认值 / 注释 / 长度都用它，保证一套视觉） */
const cellCls =
  "bg-black/30 border border-white/10 rounded-md px-2 py-1 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60";
const thCls =
  "px-1.5 py-1.5 text-left text-tiny font-semibold text-slate-400 border-b border-white/10 whitespace-nowrap";
const tdCls = "px-1 py-1 border-b border-white/5 align-middle";
/** 属性勾选框：列宽只有 ~34px，方框必须小，否则整张表会被撑松 */
const boxCls = "h-3 w-3 accent-[var(--module-accent)] cursor-pointer";

/** 「长度」列按类型给对应控件：varchar/char → 长度，decimal → 精度+小数位，enum → 取值列表 */
function TypeParamCell({
  field,
  index,
  onUpdateField,
}: {
  field: DbField;
  index: number;
  onUpdateField: (index: number, patch: Partial<DbField>) => void;
}) {
  const setType = (patch: Partial<DbField["type"]>) =>
    onUpdateField(index, { type: { ...field.type, ...patch } });
  const { base } = field.type;

  if (base === "varchar" || base === "char") {
    return (
      <input
        type="number"
        value={field.type.length ?? 255}
        onChange={(e) => setType({ length: Number(e.target.value) })}
        className={`${cellCls} w-16`}
      />
    );
  }
  if (base === "decimal") {
    return (
      <div className="flex items-center gap-1">
        <input
          type="number"
          value={field.type.precision ?? 10}
          onChange={(e) => setType({ precision: Number(e.target.value) })}
          className={`${cellCls} w-12`}
        />
        <span className="text-slate-600">,</span>
        <input
          type="number"
          value={field.type.scale ?? 2}
          onChange={(e) => setType({ scale: Number(e.target.value) })}
          className={`${cellCls} w-10`}
        />
      </div>
    );
  }
  if (base === "enum") {
    return (
      <input
        value={(field.type.values ?? []).join(",")}
        onChange={(e) =>
          setType({ values: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) })
        }
        placeholder="a,b,c"
        className={`${cellCls} w-28`}
      />
    );
  }
  return <span className="px-1 text-slate-700">—</span>;
}

export default function FieldTable({
  table,
  onUpdateField,
  onRenameField,
  onAddField,
  onRemoveField,
}: {
  table: DbTableBody;
  onUpdateField: (index: number, patch: Partial<DbField>) => void;
  /** 走后端命令级联更新关联与索引，所以是异步的 */
  onRenameField: (oldName: string, newName: string) => void;
  onAddField: () => void;
  onRemoveField: (name: string) => void;
}) {
  const { t } = useTranslation();
  const fields = table.fields ?? [];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-separate border-spacing-0">
          <thead className="sticky top-0 z-10 bg-surface-panel">
            <tr>
              <th className={`${thCls} min-w-[120px]`}>{t("dbd.colName")}</th>
              <th className={`${thCls} min-w-[104px]`}>{t("dbd.colType")}</th>
              <th className={thCls}>{t("dbd.length")}</th>
              {/* 属性列：表头 title 写清「勾上意味着什么」，缩写不再靠猜 */}
              <th className={`${thCls} w-9 text-center`} title={t("dbd.hintPk")}>
                {t("dbd.colPk")}
              </th>
              <th className={`${thCls} w-9 text-center`} title={t("dbd.hintAuto")}>
                {t("dbd.colAuto")}
              </th>
              <th className={`${thCls} w-9 text-center`} title={t("dbd.hintUnique")}>
                {t("dbd.colUnique")}
              </th>
              <th className={`${thCls} w-9 text-center`} title={t("dbd.hintNullable")}>
                {t("dbd.colNullable")}
              </th>
              <th className={`${thCls} min-w-[92px]`}>{t("dbd.colDefault")}</th>
              <th className={`${thCls} min-w-[110px]`}>{t("dbd.comment")}</th>
              <th className={thCls} />
            </tr>
          </thead>
          <tbody>
            {fields.length === 0 ? (
              <tr>
                <td colSpan={10} className="px-2 py-3 text-center text-body text-slate-600">
                  {t("dbd.noFields")}
                </td>
              </tr>
            ) : null}
            {fields.map((f, i) => (
              <tr key={`${f.name}-${i}`} className="group">
                <td className={tdCls}>
                  <input
                    defaultValue={f.name}
                    onBlur={(e) => onRenameField(f.name, e.target.value.trim())}
                    className="w-full min-w-[110px] rounded-md border border-transparent bg-transparent px-1 py-0.5 text-caption font-medium text-slate-200 focus:border-[var(--module-accent)]/50 focus:outline-none"
                  />
                </td>
                <td className={tdCls}>
                  <select
                    value={f.type.base}
                    onChange={(e) => onUpdateField(i, { type: { ...f.type, base: e.target.value } })}
                    className={`${cellCls} w-full min-w-[100px]`}
                  >
                    {BASE_TYPES.map((b) => (
                      <option key={b} value={b}>
                        {b}
                      </option>
                    ))}
                  </select>
                </td>
                <td className={tdCls}>
                  <TypeParamCell field={f} index={i} onUpdateField={onUpdateField} />
                </td>
                <td className={`${tdCls} text-center`}>
                  <input
                    type="checkbox"
                    className={boxCls}
                    checked={!!f.pk}
                    onChange={(e) => onUpdateField(i, { pk: e.target.checked })}
                  />
                </td>
                <td className={`${tdCls} text-center`}>
                  {/* 自增必须是键（MySQL / PG 都要求），所以开自增顺手把主键点上；
                      关自增不动主键 —— 用户可能本来就是复合主键里的一列。 */}
                  <input
                    type="checkbox"
                    className={boxCls}
                    checked={!!f.autoIncrement}
                    onChange={(e) =>
                      onUpdateField(i, e.target.checked ? { autoIncrement: true, pk: true } : { autoIncrement: false })
                    }
                  />
                </td>
                <td className={`${tdCls} text-center`}>
                  <input
                    type="checkbox"
                    className={boxCls}
                    checked={!!f.unique}
                    onChange={(e) => onUpdateField(i, { unique: e.target.checked })}
                  />
                </td>
                <td className={`${tdCls} text-center`}>
                  <input
                    type="checkbox"
                    className={boxCls}
                    checked={!!f.nullable}
                    onChange={(e) => onUpdateField(i, { nullable: e.target.checked })}
                  />
                </td>
                {/* 默认值：按 SQL 字面量写（字符串自己加引号），导出时原样进 DEFAULT */}
                <td className={tdCls}>
                  <input
                    value={f.default ?? ""}
                    onChange={(e) => onUpdateField(i, { default: e.target.value })}
                    placeholder={t("dbd.defaultPh")}
                    title={t("dbd.defaultValue")}
                    className={`${cellCls} w-full min-w-[88px]`}
                  />
                </td>
                <td className={tdCls}>
                  <input
                    value={f.comment ?? ""}
                    onChange={(e) => onUpdateField(i, { comment: e.target.value })}
                    placeholder={t("dbd.comment")}
                    className={`${cellCls} w-full min-w-[106px]`}
                  />
                </td>
                <td className={`${tdCls} w-8`}>
                  <button
                    onClick={() => onRemoveField(f.name)}
                    className="shrink-0 cursor-pointer p-0.5 text-slate-600 opacity-0 transition group-hover:opacity-100 hover:text-rose-400"
                    title={t("common.delete")}
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="flex shrink-0 items-center gap-3 border-t border-white/10 px-3 py-1.5">
        <button
          onClick={onAddField}
          className="flex cursor-pointer items-center gap-1 text-body text-slate-400 transition-colors hover:text-[var(--module-accent)]"
        >
          <Plus className="h-3.5 w-3.5" /> {t("dbd.addField")}
        </button>
        <span className="truncate text-micro text-slate-600">{t("dbd.fieldsHint")}</span>
      </div>
    </div>
  );
}
