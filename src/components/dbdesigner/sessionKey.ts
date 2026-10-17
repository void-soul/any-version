// 「这份设计当前绑着哪个会话」的判据 —— 纯逻辑，无 IO。
//
// 判错一次，用户看到的就是别的设计的对话；而串台不会报错、只表现为
// 「模型突然提到别的表」，极难自己发现。面板带 i18n / Tauri / React Flow
// 依赖，单测拖不进来，所以规则必须落成可断言的纯函数。
// 完整判据见 docs/plans/2026-10-17-db-design-ai-session-design.md 第二节。

export interface SessionEntry {
  id: string;
  /** 文件名（去扩展名），只为让人在列表里认得出 */
  title: string;
  /** 最近一次打开/保存的位置 —— 文件被移动后靠它认回原会话 */
  path: string;
  updatedAt: number;
}

export interface ResolveInput {
  /** 文档自带的 doc.id；外部文件与旧文件可能是空串 */
  docId: string;
  /** 当前文件路径；未保存的新文档是空串 */
  path: string;
  index: SessionEntry[];
}

export interface ResolvedSession {
  id: string;
  /** 这个 id 是本次新冒出来的（调用方据此登记索引） */
  isNew: boolean;
}

/**
 * 生成一个会话 id。字符集必须与后端 chat.rs::is_safe_session_id 一致
 * （只放行 ASCII 字母数字 _ - ，限长 128），否则保存会失败。
 *
 * 用随机后缀而不是纯时间戳：同一毫秒分配两次必须拿到不同 id，
 * 而 id 撞了意味着两份设计共用一份对话。
 */
export function createSessionId(): string {
  // 进程内自增序号（对齐后端 `file.rs::new_document_id` 用 AtomicU64 的做法）：
  // 光靠 6 位随机数是**概率**唯一，而 Task 2 刚在 Rust 侧把同类问题**确定性**修掉，
  // 两边不对称 —— id 撞了意味着两份设计共用一份对话。
  seq += 1;
  return `s_${Date.now()}_${seq.toString(36)}`;
}

let seq = 0;

/**
 * 定出这份设计该用哪个会话。优先级不能调换：
 *
 * 1. path 命中索引 → 用它。文件被移动/改名后靠这条认回原会话 ——
 *    那时 doc.id 不变、路径变了，若按 id 优先会认不出是同一份设计。
 * 2. docId 命中索引 → 用它（复制文件后沿用 id，对话跟着走）。
 * 3. docId 非空 → 用它并登记（自己存的设计文件都走这条）。
 * 4. 都没有 → 分配新 id（外部 .json 一律落到这里）。
 *
 * 第 4 条为什么必需：load_file() 不补 id（补 id 的 parse_document 只在
 * AI 那条路上），所以手写的 / AI 产出的设计文件打开后 doc.id 是空串。
 * 不分配新 id 的话，所有这类文档会共用同一份对话。
 */
export function resolveSessionId({ docId, path, index }: ResolveInput): ResolvedSession {
  // path 为空（未保存的新文档）不参与匹配：否则它会认领到索引里第一条
  // path 为空的记录 —— 也就是上一个未保存文档的会话。
  const trimmedPath = (path ?? "").trim();
  if (trimmedPath) {
    // 同一条 path 可能有多条记录（用户换过会话 → doc.id 变了 → 索引里同路径
    // 出现第二条，而后端 upsert 是 push 到尾部）。**取 updatedAt 最大的那条**：
    // 取第一条的话，用户明明新建了会话，保存后重开又会回到旧会话。
    const matches = index.filter((s) => s.path && s.path === trimmedPath);
    if (matches.length > 0) {
      const latest = matches.reduce((a, b) => ((b.updatedAt || 0) > (a.updatedAt || 0) ? b : a));
      return { id: latest.id, isNew: false };
    }
  }
  const id = (docId ?? "").trim();
  if (id) return { id, isNew: !index.some((s) => s.id === id) };
  return { id: createSessionId(), isNew: true };
}

/**
 * 从路径取一个人类可读的名字。fallback 由调用方传 i18n 文案（不硬编码中文）。
 */
export function sessionTitle(path: string, fallback: string): string {
  const trimmed = (path ?? "").trim();
  if (!trimmed) return fallback;
  const file = trimmed.split(/[\\/]/).pop() ?? "";
  const stem = file
    .replace(/\.dbdesign\.json$/i, "")
    .replace(/\.json$/i, "")
    .trim();
  // 路径就是 .json 这类怪东西时，剥完是空 —— 显示空行比显示兜底文案更糟
  return stem || fallback;
}

/**
 * 一条气泡能不能用。会话文件是用户可编辑的，手改坏了就会出现非对象、
 * 缺字段、role 不认识的值。
 *
 * 裸 as ChatMsg[] 的后果：msg.role / msg.text 全是 undefined，渲染成空白
 * 气泡、Bubble 里的分支判断走空 —— 宁可丢掉那一条，也不能让整个侧栏炸掉。
 * 丢的是用户手改坏的数据，不是我们的。
 */
export function isUsableMessage(v: unknown): boolean {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return false;
  const m = v as { id?: unknown; role?: unknown };
  if (typeof m.id !== "string" || m.id === "") return false;
  return m.role === "user" || m.role === "assistant";
}