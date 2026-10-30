import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { createLoadSeq, createSessionId, isUsableMessage, resolveSessionId, sessionTitle, type SessionEntry } from "./sessionKey";
import { type ChatMsg } from "./chatState";
import { type DbDesignDocument, type ChatTurn } from "./types";

/** Owns the DbChatSessions feature, independently of page rendering. */
interface DbChatSnapshot { msgs: ChatMsg[]; history: ChatTurn[]; clarifyRound: number; }

export function useDbChatSessions({ doc, path, readChatSnapshot, applyChatSnapshot, resetChatEphemeral, assignDocumentIdentity }: {
  doc: DbDesignDocument | null;
  path: string;
  readChatSnapshot: () => DbChatSnapshot;
  applyChatSnapshot: (chat: DbChatSnapshot) => void;
  resetChatEphemeral: () => void;
  assignDocumentIdentity: (id: string, source?: DbDesignDocument) => void;
}) {
  const { t } = useTranslation();

  /**
   * 当前设计绑着的会话 id。
   *
   * 它就是 `doc.id`（外部文件缺 id 时会在 loadChatForDoc 里分配一个补上），
   * 这里另存一份是为了**读盘期间 doc 还是原值**时也能拿到正确的 key 去写。
   */
  const [sessionId, setSessionId] = useState("");

  /** 会话索引（弹框列表 + resolveSessionId 的输入） */
  const [sessions, setSessions] = useState<SessionEntry[]>([]);

  /**
   * 「读盘中」—— 换文档 / 切会话后，会话要从磁盘异步读回来，这段窗口里
   * `sessionId` 与 `aiHistory` 还是上一份的。用户此时回车就是把「新表 + 旧对话」
   * 发给模型、并把这一轮存进旧会话，**串台换个入口复活**。
   *
   * 所以这段时间禁用发送。`chatLoadSeq` 同时是那道竞态闸门
   * （后发起的接管先发起的），状态机与不变量见 `sessionKey.ts::createLoadSeq`。
   */
  const [chatLoading, setChatLoading] = useState(false);

  const chatLoadSeqRef = useRef(createLoadSeq((loading) => setChatLoading(loading)));

  // ── 文件 ──
  /**
   * 把某个会话登记进索引（upsert，不是整份覆盖）。
   *
   * 为什么 upsert：前端手里那份 `sessions` 可能已经过期（另一个窗口改过），
   * 整份写回会把别人的条目抹掉。交给后端按 id 合并。
   */
  const touchSession = useCallback(
    async (id: string, filePath: string) => {
      if (!id) return;
      try {
        const title = sessionTitle(filePath, doc?.name?.trim() || t("dbd.untitled"));
        const next = await invoke<{ sessions: SessionEntry[] }>("dbd_upsert_session_entry", {
          id,
          title,
          path: filePath,
        });
        setSessions(next.sessions);
      } catch {
        // 索引写不进去不该挡住对话：顶多列表里少一条，对话本身还在
      }
    },
    [doc?.name, t],
  );

  /**
   * **唯一的会话写盘出口**：`runAi` 成功轮次与切换文档前的 `saveChatNow` 都走它。
   *
   * 两件事一起做，缺一不可：
   * 1. 写正文 `chats/<id>.json`；
   * 2. `touchSession` 刷新索引里的 `updatedAt` —— 少了它，列表排序停在「最近打开」
   *    而不是「最近活跃」，而且**删掉当前会话后再聊一轮会把文件写回来、
   *    索引里却没有**，于是这个会话在列表里彻底消失（用户看得见的孤儿）。
   *
   * 写盘失败一律静默：对话照常显示，只是没存住 —— 不值得为缓存失败打断界面。
   */
  const commitChat = useCallback(
    async (chat: { msgs: ChatMsg[]; history: ChatTurn[]; clarifyRound: number }) => {
      const id = sessionId;
      if (!id) return;
      try {
        await invoke("dbd_save_chat", {
          id,
          chat: {
            chatMsgs: chat.msgs,
            aiHistory: chat.history,
            aiClarifyRound: chat.clarifyRound,
          },
        });
        void touchSession(id, path);
      } catch {
        // 同上：存不住不是致命错误
      }
    },
    [sessionId, path, touchSession],
  );

  /** 把当前对话写进当前会话（切换文档前调，避免这一轮的对话随组件状态丢掉）。 */
  const saveChatNow = useCallback(async () => {
    if (!sessionId) return;
    await commitChat(readChatSnapshot());
  }, [sessionId, commitChat, readChatSnapshot]);

  /**
   * 换文档时读回它自己的会话。
   *
   * **取代了原来那串手写的清空 setter**（`setAiHistory([])` / `setAiClarify(null)` …）：
   * 读取结果直接覆盖，语义上更对，也就不会再像 `chatMsgs` 那样漏掉一个字段
   * （漏掉的后果就是「新建文档后还留着上一份的对话」）。
   */
  const loadChatForDoc = useCallback(
    async (d: DbDesignDocument, filePath: string) => {
      const token = chatLoadSeqRef.current.begin();
      let index: SessionEntry[] = [];
      try {
        const got = await invoke<{ sessions: SessionEntry[] }>("dbd_get_session_index");
        index = got.sessions ?? [];
      } catch {
        // 读不到索引就按「没有记录」处理，仍然会分配一个新 id
      }
      const r = resolveSessionId({ docId: d.id, path: filePath, index });
      let chat: { chatMsgs?: unknown[]; aiHistory?: unknown[]; aiClarifyRound?: number } = {};
      try {
        chat = await invoke("dbd_get_chat", { id: r.id });
      } catch {
        // 会话读不出来 → 当成空会话（后端已做同样兜底，这里防的是 invoke 本身失败）
      }
      // 期间用户又切了文档 → 这次结果作废
      if (!chatLoadSeqRef.current.alive(token)) return;

      setSessionId(r.id);
      applyChatSnapshot({ msgs: (chat.chatMsgs ?? []).filter(isUsableMessage) as ChatMsg[], history: (chat.aiHistory ?? []) as ChatTurn[], clarifyRound: chat.aiClarifyRound ?? 0 });
      // 追问状态不进会话缓存：它只在单次追问流程内有意义
      resetChatEphemeral();
      // 外部文件没有 id 而被分配了新 id → 写回文档，文档因此变脏。
      // 用函数式更新并校验来源：那两次 IPC 期间用户可能拖过节点，
      // 拿调用时刻的整份快照回去覆盖会把他的拖动回滚掉。
      if (d.id !== r.id) {
        assignDocumentIdentity(r.id, d);
      }
      // 索引以 touchSession 的返回为准（它是后端给的权威列表）。
      // 刻意**不**在这里 setSessions(index)：读索引失败时 index 是空数组，
      // 那会把内存里已有的列表清空 —— 一次读失败让用户看到「会话全没了」。
      void touchSession(r.id, filePath);
      // 读到会话、状态已归位 → 「读盘中」解除。上面那条 `return` 是被后继者
      // 接管的情况，此时**不** settle：收尾归接管者管（见 createLoadSeq 的契约）。
      chatLoadSeqRef.current.settle(token);
    },
    [touchSession],
  );

  /**
   * 切到某个会话。
   *
   * **改 `doc.id` → 文档变脏**：换了个身份就该落盘，否则下次打开又认回旧会话。
   * **旧会话的内容不动** —— 切走时已经存过了。
   *
   * **刻意不清输入草稿**（只有 `newDoc` / `openFile` 清）：草稿只在**换文档**时
   * 才会"主动说错话"—— 它写的是另一批表。而会话切换不改表，草稿仍然切题；
   * `sessionId` 与 `aiHistory` 在这里已被正确刷新，所以保留草稿**不构成串台**。
   * 清掉反而会吃掉用户正在写的东西。
   */
  const switchSession = useCallback(
    async (id: string) => {
      if (!doc || id === sessionId) return;
      // 领 token 作废在飞的读取：否则 openFile 尚未返回的那次 loadChatForDoc
      // 会在切换之后才到，把用户刚切好的会话又回滚回去（设计文档第三节的要求）。
      const token = chatLoadSeqRef.current.begin();
      void saveChatNow();
      assignDocumentIdentity(id);
      setSessionId(id);
      try {
        const chat = await invoke<{ chatMsgs?: unknown[]; aiHistory?: unknown[]; aiClarifyRound?: number }>(
          "dbd_get_chat",
          { id },
        );
        if (!chatLoadSeqRef.current.alive(token)) return;
        applyChatSnapshot({ msgs: (chat.chatMsgs ?? []).filter(isUsableMessage) as ChatMsg[], history: (chat.aiHistory ?? []) as ChatTurn[], clarifyRound: chat.aiClarifyRound ?? 0 });
      } catch {
        if (!chatLoadSeqRef.current.alive(token)) return;
        applyChatSnapshot({ msgs: [], history: [], clarifyRound: 0 });
      }
      resetChatEphemeral();
      void touchSession(id, path);
      chatLoadSeqRef.current.settle(token);
    },
    [doc, sessionId, path, saveChatNow, touchSession],
  );

  /** 新建会话：生成新 id 并切过去（对话为空）。旧会话定格不动。 */
  const createSession = useCallback(() => {
    if (!doc) return;
    const id = createSessionId();
    // 同样要作废在飞的读取：不然它在返回后会把刚建的新会话 id 覆盖回去。
    // 它是同步完成的（不读盘），所以自己 settle —— 不 settle 就把「读盘中」卡死了。
    const token = chatLoadSeqRef.current.begin();
    void saveChatNow();
    assignDocumentIdentity(id);
    setSessionId(id);
    applyChatSnapshot({ msgs: [], history: [], clarifyRound: 0 });
    resetChatEphemeral();
    void touchSession(id, path);
    chatLoadSeqRef.current.settle(token);
  }, [doc, path, saveChatNow, touchSession]);

  /**
   * 删掉一个会话的缓存。
   *
   * **删的是当前会话时不报错、不补救**：文档的 `doc.id` 还在，下次
   * `resolveSessionId` 会走第 3 条重新登记它、读到空 —— 对话自然变空，文档不脏。
   * (删完顺手把列表里那条摘掉，否则弹框里还留着一条点不动的死条目。)
   */
  const deleteSession = useCallback(
    async (id: string) => {
      try {
        await invoke("dbd_delete_session", { id });
      } catch {
        return;
      }
      setSessions((prev) => prev.filter((s) => s.id !== id));
      if (id !== sessionId) return;
      const token = chatLoadSeqRef.current.begin();
      try {
        const chat = await invoke<{ chatMsgs?: unknown[]; aiHistory?: unknown[]; aiClarifyRound?: number }>(
          "dbd_get_chat",
          { id },
        );
        if (!chatLoadSeqRef.current.alive(token)) return;
        applyChatSnapshot({ msgs: (chat.chatMsgs ?? []).filter(isUsableMessage) as ChatMsg[], history: (chat.aiHistory ?? []) as ChatTurn[], clarifyRound: chat.aiClarifyRound ?? 0 });
      } catch {
        if (!chatLoadSeqRef.current.alive(token)) return;
        applyChatSnapshot({ msgs: [], history: [], clarifyRound: 0 });
      }
      // 追问状态一并清：卡指向的是一条已经不存在的气泡，留着会点不动
      resetChatEphemeral();
      chatLoadSeqRef.current.settle(token);
    },
    [sessionId],
  );
  return {
    sessionId,
    sessions,
    chatLoading,
    touchSession,
    commitChat,
    saveChatNow,
    loadChatForDoc,
    switchSession,
    createSession,
    deleteSession,
  };
}
