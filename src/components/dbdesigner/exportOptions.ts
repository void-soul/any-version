// 导出弹框的**纯逻辑**部分：选项类型 + 默认值 + 目标路径预测。
//
// 为什么单独拆一个文件：这两个函数没有 React、没有 i18n，可以在测试里直接断言；
// 而它们恰恰是最容易错的地方 —— 路径预测必须与后端 `file.rs::sibling_path`
// 的推导规则一致，前端显示 `A.sql` 后端却写到 `B.sql`，用户会以为导出坏了。
//
// 展示与交互在 ExportDialog.tsx。

/** 导出目标：写到设计文件旁边 / 进剪贴板 / 让用户挑个路径 */
export type ExportTarget = "file" | "clipboard" | "saveAs";

export interface ExportChoice {
  target: ExportTarget;
  /** 是否输出外键约束（对应后端 ExportOptions.foreignKeys） */
  foreignKeys: boolean;
}

/**
 * 默认选项：带外键 + 有落盘过就写文件旁边、否则进剪贴板。
 *
 * 目标默认跟着「有没有落盘过」走，是为了和旧按钮的行为一致（没保存过的设计
 * 以前就是复制到剪贴板）；而`foreignKeys` 默认 true ——「不带外键」是每次导出的
 * 刻意选择，不该因为上一次这么选过就一直沿用。
 */
export function defaultExportChoice(hasPath: boolean): ExportChoice {
  return {
    target: hasPath ? "file" : "clipboard",
    foreignKeys: true,
  };
}

/**
 * 预测「写入文件」会落到哪：`订单库.dbdesign.json` → `订单库.sql`。
 *
 * **必须与后端 `sibling_path` 同规则**（同目录、同主名、换后缀；先剥
 * `.dbdesign.json` 再剥 `.json`）。这里只是给用户看的预览，不参与实际写入，
 * 所以宁可显示得直白，也不要假装很确定。
 */
export function predictSqlPath(designPath: string | null): string {
  if (!designPath) return "";
  const file = designPath.split(/[\\/]/).pop() ?? designPath;
  const stem = file.replace(/\.dbdesign\.json$/i, "").replace(/\.json$/i, "") || "design";
  const dir = designPath.slice(0, designPath.length - file.length);
  return `${dir}${stem}.sql`;
}