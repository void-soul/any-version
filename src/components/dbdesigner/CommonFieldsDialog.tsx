// 公共字段配置：勾选启用哪些、统一改名字。
//
// 需求（用户原话）：「理论上每个表都应该有创建修改删除时间和人，以及备注、是否逻辑删除，
// 这些字段的名称应该统一……但这样未免太武断」—— 所以是**配置**，不是硬规则。
//
// 三条设计约束（与后端 common_fields.rs 对齐）：
//   · 存**完整定义**（自包含），不是「勾了哪几个 key」；
//   · 名字统一在一处改，导出时自动附加到每张表末尾；
//   · **表里已有同名字段时以表自己的为准**（不合并、不覆盖）—— 所以在这里改名
//     **不会**改写用户已有的列，想让已有列也统一得自己去改表（那是有意义的重构）。
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { SharedModal } from "../shared/Modal";
import type { DbField } from "./types";

/** 与后端 `builtin_common_fields()` 一一对应（顺序即 UI 顺序）。 */
const BUILTIN: { key: string; name: string; base: string; length: number | null; comment: string }[] = [
  { key: "createdAt", name: "created_at", base: "datetime", length: null, comment: "创建时间" },
  { key: "updatedAt", name: "updated_at", base: "datetime", length: null, comment: "修改时间" },
  { key: "deletedAt", name: "deleted_at", base: "datetime", length: null, comment: "删除时间（可空）" },
  { key: "createdBy", name: "created_by", base: "bigint", length: null, comment: "创建人（用户 id，可空）" },
  { key: "updatedBy", name: "updated_by", base: "bigint", length: null, comment: "修改人（用户 id，可空）" },
  { key: "deletedBy", name: "deleted_by", base: "bigint", length: null, comment: "删除人（用户 id，可空）" },
  { key: "remark", name: "remark", base: "varchar", length: 500, comment: "备注" },
  { key: "isDeleted", name: "is_deleted", base: "boolean", length: null, comment: "是否逻辑删除" },
];

function makeField(key: string, name: string): DbField {
  const b = BUILTIN.find((x) => x.key === key)!;
  return {
    name,
    type: { base: b.base, length: b.length, precision: null, scale: null, unsigned: false, values: [] },
    // 删除时间与「谁」可空（还没发生的事），其余不空
    nullable: b.key === "deletedAt" || b.key.endsWith("By") || b.key === "remark",
    // 一律按字符串存：导出层按方言归一（boolean → MySQL/SQLite 0、PG FALSE）
    default:
      b.key === "createdAt" || b.key === "updatedAt"
        ? "CURRENT_TIMESTAMP"
        : b.key === "isDeleted"
          ? "false"
          : null,
    comment: b.comment,
    pk: false,
    autoIncrement: false,
    unique: false,
  };
}

/**
 * 存的是完整定义、没有 key，所以要反查它属于哪个内置项。
 *
 * **先按 comment 匹配**（每个内置项的 comment 唯一），再退回按名字 ——
 * 只按名字匹配的话，用户把 created_at 改名成 create_time 就再也对不上了；
 * 只按类型匹配则更糟：三个 datetime 字段会互相串（这是第一版的写法，实测才发现）。
 */
function keyOfField(f: DbField): string | undefined {
  const byComment = BUILTIN.find((b) => b.comment === f.comment);
  if (byComment) return byComment.key;
  return BUILTIN.find((b) => b.name === f.name.trim())?.key;
}

/** 首次打开（或当前为空）时给一份推荐清单：两个时间 + 创建/修改人，其余交给你勾。 */
function initialFields(current: DbField[] | undefined): DbField[] {
  if (current && current.length > 0) return current;
  return ["createdAt", "updatedAt", "createdBy", "updatedBy"].map((k) => makeField(k, BUILTIN.find((b) => b.key === k)!.name));
}

export default function CommonFieldsDialog({
  open,
  value,
  onClose,
  onSave,
}: {
  open: boolean;
  value: DbField[] | undefined;
  onClose: () => void;
  onSave: (fields: DbField[]) => void;
}) {
  const { t } = useTranslation();
  const [fields, setFields] = useState<DbField[]>(() => initialFields(value));
  if (!open) return null;

  const fieldOf = (key: string) => fields.find((f) => keyOfField(f) === key);
  const toggle = (key: string, on: boolean) => {
    if (on) {
      if (!fieldOf(key)) setFields((prev) => [...prev, makeField(key, BUILTIN.find((b) => b.key === key)!.name)]);
    } else {
      setFields((prev) => prev.filter((f) => keyOfField(f) !== key));
    }
  };
  const rename = (key: string, name: string) => {
    setFields((prev) => prev.map((f) => (keyOfField(f) === key ? { ...f, name } : f)));
  };

  return (
    <SharedModal
      open={open}
      onClose={onClose}
      title={t("dbd.commonFieldsTitle")}
      width={560}
      footer={
        <div className="flex items-center justify-between gap-2">
          <button
            onClick={() => setFields([])}
            className="cursor-pointer rounded-ctl px-2 py-1.5 text-micro text-slate-400 hover:bg-white/5 hover:text-slate-200"
          >
            {t("dbd.commonFieldsClear")}
          </button>
          <div className="flex gap-2">
            <button onClick={onClose} className="cursor-pointer rounded-ctl px-3 py-1.5 text-body text-slate-300 hover:bg-white/5">
              {t("dbd.exportCancel")}
            </button>
            <button
              onClick={() => {
                onSave(fields);
                onClose();
              }}
              className="cursor-pointer rounded-ctl bg-[var(--module-accent)] px-3 py-1.5 text-body font-semibold text-white"
            >
              {t("dbd.commonFieldsSave")}
            </button>
          </div>
        </div>
      }
    >
      <p className="mb-2 text-micro text-slate-500">{t("dbd.commonFieldsHint")}</p>
      <div className="max-h-[46vh] space-y-1 overflow-y-auto pr-0.5">
        {BUILTIN.map((b) => {
          const f = fieldOf(b.key);
          return (
            <label
              key={b.key}
              className={`flex items-center gap-2 rounded-ctl border px-2 py-1.5 transition ${
                f ? "border-[var(--module-accent)] bg-white/[0.04]" : "border-white/10 hover:bg-white/[0.02]"
              }`}
            >
              <input type="checkbox" checked={!!f} onChange={(e) => toggle(b.key, e.target.checked)} />
              <input
                value={f?.name ?? b.name}
                disabled={!f}
                onChange={(e) => rename(b.key, e.target.value)}
                className="w-32 shrink-0 rounded bg-white/5 px-1.5 py-0.5 font-mono text-micro text-slate-100 outline-none disabled:opacity-40"
              />
              <span className="min-w-0 flex-1 truncate text-micro text-slate-400">
                {b.length ? `${b.base}(${b.length})` : b.base} · {b.comment}
              </span>
            </label>
          );
        })}
      </div>
      <p className="mt-2 text-micro text-slate-500">{t("dbd.commonFieldsNote")}</p>
    </SharedModal>
  );
}
