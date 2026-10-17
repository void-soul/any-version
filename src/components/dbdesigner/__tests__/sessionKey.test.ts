import { describe, it, expect } from "vitest";

import { resolveSessionId, sessionTitle, createSessionId, isUsableMessage } from "../sessionKey";

/**
 * 「这份设计当前绑着哪个会话」的判据。
 *
 * 它是整个功能的地基：**判错一次，对话就串到别的设计上**，而串台不会报错、
 * 只表现为「模型突然提到别的表」，极难自己发现。
 *
 * 顺序不能调换 —— `path` 必须优先于 `doc.id`：文件被移动/改名时 `doc.id` 不变、
 * 路径变了，若按 id 优先会认不出「这是同一份设计」。
 */
describe("resolveSessionId", () => {
  const entry = (id: string, path: string, title = "t") => ({ id, title, path, updatedAt: 1 });

  it("路径能匹配上索引时优先用它 —— 文件移动/改名后要认回原会话", () => {
    const index = [entry("s_old", "C:\\新位置\\订单库.dbdesign.json")];
    const r = resolveSessionId({ docId: "dbd_999", path: "C:\\新位置\\订单库.dbdesign.json", index });
    expect(r.id).toBe("s_old");
    expect(r.isNew).toBe(false);
  });

  it("路径匹配不上时回落到 docId（复制文件后沿用 id → 对话跟着走）", () => {
    const index = [entry("dbd_999", "C:\\a.dbdesign.json")];
    const r = resolveSessionId({ docId: "dbd_999", path: "C:\\b.dbdesign.json", index });
    expect(r.id).toBe("dbd_999");
    expect(r.isNew).toBe(false);
  });

  it("docId 也没登记过 → 用 docId，但标记为新建（调用方据此登记索引）", () => {
    const r = resolveSessionId({ docId: "dbd_new", path: "C:\\c.dbdesign.json", index: [] });
    expect(r).toEqual({ id: "dbd_new", isNew: true });
  });

  it("外部文件没有 id → 分配一个新 id（否则所有外部文件会共用一份对话）", () => {
    // 最危险的一条：load_file 不补 id（补 id 的 parse_document 只在 AI 那条路上），
    // 所以手写的 / AI 产出的 .json 打开后 id 一律是空串。
    const r = resolveSessionId({ docId: "", path: "C:\\d.dbdesign.json", index: [] });
    expect(r.isNew).toBe(true);
    expect(r.id).not.toBe("");
    expect(r.id).not.toBe("undefined");
  });

  it("docId 是空白串时按「没有」处理", () => {
    const r = resolveSessionId({ docId: "   ", path: "C:\\d.dbdesign.json", index: [] });
    expect(r.id).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("未保存的新文档（path 为空）→ 用 docId", () => {
    expect(resolveSessionId({ docId: "dbd_fresh", path: "", index: [] })).toEqual({
      id: "dbd_fresh",
      isNew: true,
    });
  });

  it("未保存的新文档 docId 也是空 → 照样分配一个（不能出现空 key）", () => {
    const r = resolveSessionId({ docId: "", path: "", index: [] });
    expect(r.id).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("docId 已在索引里 → isNew 为 false（别重复登记）", () => {
    const index = [entry("dbd_1", "C:\\x.dbdesign.json")];
    const r = resolveSessionId({ docId: "dbd_1", path: "C:\\y.dbdesign.json", index });
    expect(r.isNew).toBe(false);
  });

  it("索引里有两条同 id 的坏数据时，取第一条且不崩", () => {
    const index = [entry("dup", "C:\\p.dbdesign.json", "先"), entry("dup", "C:\\q.dbdesign.json", "后")];
    const r = resolveSessionId({ docId: "", path: "C:\\p.dbdesign.json", index });
    expect(r.id).toBe("dup");
  });

  it("同一路径有多条记录时取 updatedAt 最大的那条（用户新建会话后保存重开不能回退）", () => {
    // 后端 upsert 是 push 到尾部，所以「同路径的多条」里新的在后面。
    // 取第一条的话，用户明明建了新会话，保存后重开又回到旧会话。
    const index = [
      { id: "s_old", title: "旧会话", path: "C:\\a.dbdesign.json", updatedAt: 100 },
      { id: "s_new", title: "新会话", path: "C:\\a.dbdesign.json", updatedAt: 200 },
    ];
    expect(resolveSessionId({ docId: "", path: "C:\\a.dbdesign.json", index }).id).toBe("s_new");
    // updatedAt 相同（或都缺）时不能崩，退回第一条
    const same = [
      { id: "x1", title: "1", path: "C:\\b.json", updatedAt: 0 },
      { id: "x2", title: "2", path: "C:\\b.json", updatedAt: 0 },
    ];
    expect(resolveSessionId({ docId: "", path: "C:\\b.json", index: same }).id).toBe("x1");
  });

  it("索引里 path 为空的条目不该被空路径匹配上", () => {
    // 未保存的文档也会进索引（path 为空）。若不排除，任何未保存文档都会
    // 认领到索引里第一条 path 为空的记录 —— 也就是上一个未保存文档的会话。
    const index = [entry("s_old", "", "旧会话")];
    const r = resolveSessionId({ docId: "dbd_fresh", path: "", index });
    expect(r.id).toBe("dbd_fresh");
  });

  it("生成的 id 只含安全字符（后端会直接拿它当文件名）", () => {
    const r = resolveSessionId({ docId: "", path: "C:\\d.dbdesign.json", index: [] });
    expect(r.id).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("连续分配多个 id 不重复（否则多份文档会共用一份对话）", () => {
    const ids = new Set(Array.from({ length: 200 }, () => createSessionId()));
    expect(ids.size).toBe(200);
  });

  it("createSessionId 出来的 id 能通过长度上限（后端限 128）", () => {
    for (let i = 0; i < 50; i++) expect(createSessionId().length).toBeLessThanOrEqual(128);
  });
});

describe("sessionTitle", () => {
  const FALLBACK = "未保存";

  it("从路径取文件名并去掉 .dbdesign.json", () => {
    expect(sessionTitle("C:\\设计\\订单库.dbdesign.json", FALLBACK)).toBe("订单库");
  });

  it("兼容正斜杠与无扩展名", () => {
    expect(sessionTitle("/home/me/orders.dbdesign.json", FALLBACK)).toBe("orders");
    expect(sessionTitle("C:\\设计\\订单库", FALLBACK)).toBe("订单库");
    expect(sessionTitle("C:\\设计\\a.b.dbdesign.json", FALLBACK)).toBe("a.b");
  });

  it("扩展名匹配忽略大小写（.DBDESIGN.JSON 也是设计文件）", () => {
    expect(sessionTitle("C:\\a\\订单库.DBDESIGN.JSON", FALLBACK)).toBe("订单库");
  });

  it("只有 .json 后缀时也剥掉（用户可能存成了 order.json）", () => {
    expect(sessionTitle("C:\\a\\order.json", FALLBACK)).toBe("order");
  });

  it("路径为空 → 用兜底文案", () => {
    expect(sessionTitle("", FALLBACK)).toBe(FALLBACK);
    expect(sessionTitle("   ", FALLBACK)).toBe(FALLBACK);
  });

  it("剥完扩展名还空（路径是 .json）→ 用兜底文案，不显示成空行", () => {
    expect(sessionTitle("C:\\a\\.json", FALLBACK)).toBe(FALLBACK);
  });
});

/**
 * 会话文件是被用户可编辑的：手改坏了、或者中途换了格式，都可能出现
 * 非对象的气泡。裸 `as ChatMsg[]` 会让 `msg.role` / `msg.text` 全是 undefined，
 * 渲染成空白气泡还可能崩 —— 宁可丢掉那一条，也不能让整个侧栏炸掉。
 */
describe("isUsableMessage", () => {
  it("对象且有 id 与合法 role 才算一条气泡", () => {
    expect(isUsableMessage({ id: "m1", role: "user", text: "hi" })).toBe(true);
    expect(isUsableMessage({ id: "m1", role: "assistant" })).toBe(true);
  });

  it("非对象、null、数组一律不算", () => {
    for (const bad of ["oops", 42, null, undefined, [], true]) {
      expect(isUsableMessage(bad)).toBe(false);
    }
  });

  it("缺 id 或 role 的不算（渲染与 React key 都依赖它们）", () => {
    expect(isUsableMessage({ role: "user", text: "x" })).toBe(false);
    expect(isUsableMessage({ id: "m1", text: "x" })).toBe(false);
  });

  it("role 必须是 user / assistant —— 别的值前端分支处理不了", () => {
    expect(isUsableMessage({ id: "m1", role: "system" })).toBe(false);
    expect(isUsableMessage({ id: "m1", role: "user" })).toBe(true);
    expect(isUsableMessage({ id: "m1", role: "assistant" })).toBe(true);
  });
});