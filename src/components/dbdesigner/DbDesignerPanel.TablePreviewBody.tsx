import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { typeLabel, type DbField } from "./types";
/**
 * 表列表悬浮预览的正文：注释 + 完整字段清单 + 统计。
 *
 * **用普通 DOM 而不是 markdown**：字段天然是三列对齐的表（名 / 类型 / 标记 / 注释），
 * 走 markdown 反而要 `await invoke` 调后端的 `dbd_export_outline`（那是整文档级的、
 * 异步 IPC，每次悬浮发一次不可接受），或者把那段 Rust 逻辑复制一份到 TS。
 */
export function TablePreviewBody({ row }: { row: { comment: string; fields: DbField[]; indexCount: number; relCount: number } }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5">
      {row.comment && <p className="text-micro leading-relaxed text-slate-400">{row.comment}</p>}
      {row.fields.length === 0 ? (
        <p className="text-micro text-slate-600">{t("dbd.viewNoFields")}</p>
      ) : (
        <div className="space-y-px">
          {/* 字段用**真表格**展示，不用 flex 行。
              flex 行里字段名长短不一，类型与注释就跟着参差不齐，扫一眼对不上是哪一列；
              表格靠固定列宽天然对齐，跨表看字段清单时能直接按列比较
              （用户要求）。宽度按 320px 气泡分配：名字可截断、类型与注释各留够，
              完整内容都在 title 里。 */}
          <table className="w-full table-fixed text-micro">
            <colgroup>
              <col className="w-[38%]" />
              <col className="w-[26%]" />
              <col className="w-[11%]" />
              <col />
            </colgroup>
            <tbody>
              {row.fields.map((f) => (
                <tr key={f.name} className="align-top odd:bg-white/[0.015]">
                  <td
                    className="truncate py-0.5 pr-1.5 font-mono text-slate-200"
                    title={f.name}
                  >
                    {f.name}
                  </td>
                  <td className="truncate py-0.5 pr-1.5 font-mono text-slate-500" title={typeLabel(f.type)}>
                    {typeLabel(f.type)}
                  </td>
                  <td className="whitespace-nowrap py-0.5 pr-1.5">
                    {f.pk ? (
                      <span className="text-amber-400/90">PK</span>
                    ) : f.unique ? (
                      <span className="text-emerald-400/80">UQ</span>
                    ) : null}
                  </td>
                  <td className="truncate py-0.5 text-slate-500" title={f.comment || undefined}>
                    {f.comment}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <div className="flex items-center gap-2 border-t border-white/10 pt-1.5 text-micro text-slate-500">
        <span>{t("dbd.fieldCount", { count: row.fields.length })}</span>
        {row.indexCount > 0 && <span>{t("dbd.indexCount", { count: row.indexCount })}</span>}
        {row.relCount > 0 && <span>{t("dbd.relCount", { count: row.relCount })}</span>}
      </div>
    </div>
  );
}
