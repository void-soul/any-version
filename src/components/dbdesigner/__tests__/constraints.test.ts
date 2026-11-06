import { describe, expect, it } from "vitest";
import { FK_ACTIONS } from "../DbDesignerPanel.model";

/** 与 Rust 真源、前端镜像、各编辑器做静态对齐：防止「后端加了字段、前端读 undefined」。
 *  glob 必须限定目录，扫全仓库会把 node_modules / 大文件一起载入。 */
const TS = import.meta.glob<string>("/src/components/dbdesigner/*.{ts,tsx}", {
    query: "?raw", import: "default", eager: true,
});
const RS = import.meta.glob<string>("/src-tauri/src/commands/db_designer/models.rs", {
    query: "?raw", import: "default", eager: true,
});
const raw = (path: string): string => TS[path] ?? RS[path] ?? "";

const MODELS = raw("/src-tauri/src/commands/db_designer/models.rs");
const TYPES = raw("/src/components/dbdesigner/types.ts");
const REL_EDITOR = raw("/src/components/dbdesigner/DbRelationsEditor.tsx");
const CHECKS_EDITOR = raw("/src/components/dbdesigner/DbChecksEditor.tsx");
const PANEL = raw("/src/components/dbdesigner/DbDesignerPanel.tsx");
const EDITING = raw("/src/components/dbdesigner/useDbDocumentEditing.tsx");

describe("数据库设计器约束契约", () => {
    it("前端 FK_ACTIONS 与 Rust 白名单逐项一致", () => {
        const match = MODELS.match(/pub const FK_ACTIONS: \[&str; \d+\] = \[([^\]]+)\]/);
        expect(match, "models.rs 应声明 FK_ACTIONS").not.toBeNull();
        const rust = [...match![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
        expect(FK_ACTIONS).toEqual(rust);
    });

    it("types.ts 镜像了 constraintName 与具名 DbCheck", () => {
        expect(TYPES).toContain("constraintName?: string");
        expect(TYPES).toMatch(/export interface DbCheck\b/);
    });

    it("关联编辑器可编辑约束名与 ON UPDATE", () => {
        expect(REL_EDITOR).toContain("constraintName");
        expect(REL_EDITOR).toContain("{ onUpdate:");
    });

    it("CHECK 编辑器已实现并接入 dock 页签", () => {
        expect(CHECKS_EDITOR).toContain("updateCheck");
        expect(EDITING).toContain("addCheck");
        expect(PANEL).toContain('dockTab === "checks"');
    });
});
