import { describe, expect, it } from "vitest";

import { defaultExportChoice, predictSqlPath } from "../exportOptions";

describe("导出选项", () => {
  it("每次打开都用默认：带外键；目标跟着有没有落盘过走", () => {
    expect(defaultExportChoice(true)).toEqual({ target: "file", foreignKeys: true });
    // 没落盘过时不能默认选「写入文件」——那会导出失败
    expect(defaultExportChoice(false)).toEqual({ target: "clipboard", foreignKeys: true });
  });

  /**
   * 路径预测必须与后端 `file.rs::sibling_path` 同规则：同目录、同主名、换后缀。
   * 前端显示 A.sql 而后端写到 B.sql，用户会以为导出坏了。
   */
  it("预测的 SQL 路径与后端 sibling_path 同规则", () => {
    expect(predictSqlPath("C:\\db\\订单库.dbdesign.json")).toBe("C:\\db\\订单库.sql");
    expect(predictSqlPath("/home/x/订单库.dbdesign.json")).toBe("/home/x/订单库.sql");
    // 后缀退化情形：不是 .dbdesign.json 时也要剥掉 .json
    expect(predictSqlPath("/tmp/design.json")).toBe("/tmp/design.sql");
    // 没有路径时给空串（弹框里据此禁用「写入文件」并显示提示）
    expect(predictSqlPath(null)).toBe("");
    // 主名被剥空时兜底成 design（与后端 unwrap_or("design") 对齐）
    expect(predictSqlPath("/tmp/.dbdesign.json")).toBe("/tmp/design.sql");
  });
});