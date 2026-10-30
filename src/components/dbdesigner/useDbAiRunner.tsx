import { resolveModelChoice } from "./modelChoice";
import type { FieldRowMode } from "./types";
import { useEffect, useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { mergeDecisions, trimHistory } from "./aiMemory";
import { modelChoiceSettings } from "./modelChoice";
import { computeLayout, type LayoutMode } from "./layout";
import { patchCounts, patchLast, startTurn, type ChatMsg } from "./chatState";
import { useEventBufferSnapshot } from "../../utils/eventBuffer";
import { toast } from "../shared/Toast";
import { type DbDesignDocument, type AiOutcome, type ChatTurn } from "./types";
import { AI_STREAM_COLS, dbdAiPartialBuffer, dbdAiProgressBuffer, type AiProviderLike } from "./DbDesignerPanel.model";

/** Owns the DbAiRunner feature, independently of page rendering. */
export function useDbAiRunner({ doc, chatLoading, applyGeneratedDoc, fitCanvas, layoutMode, fieldRows, commitChat }: {
  doc: DbDesignDocument | null;
  chatLoading: boolean;
  applyGeneratedDoc: (next: DbDesignDocument, options?: { resetPositions?: boolean }) => void;
  fitCanvas: (clearSelection?: boolean) => void;
  layoutMode: LayoutMode;
  fieldRows: FieldRowMode;
  commitChat: (chat: { msgs: ChatMsg[]; history: ChatTurn[]; clarifyRound: number; }) => Promise<void>;
}) {
  const { t } = useTranslation();

  const [aiText, setAiText] = useState("");

  const [aiBusy, setAiBusy] = useState(false);

  /** 本轮是主动停止的：控制台收尾显示「已停止」而不是「已完成」 */
  const [, setAiAborted] = useState(false);

  const [, setAiErr] = useState("");

  /**
   * 会话历史（滚动窗口，最近 3 轮）。
   *
   * **不进设计文件**：它是对话过程的脚手架，不是设计内容。放进文件会让它越滚越大、
   * 分享文件时把两个 AI 的来回对话也发给了对方。真正的记忆是 `doc.decisions`。
   */
  const [aiHistory, setAiHistory] = useState<ChatTurn[]>([]);

  /**
   * 对话气泡（临时，关掉面板就丢）。
   *
   * 为什么不进设计文件：它是对话过程的脚手架，不是设计内容，放进文件会让它越滚越大。
   * 真正的长期记忆是 `doc.decisions`（在顶部那个按钮里）。
   */
  const [chatMsgs, setChatMsgs] = useState<ChatMsg[]>([]);

  /** 模型停下来问问题时的状态（非空 = 交洽卡片应渲染出来） */
  const [, setAiClarify] = useState<{ question: string; options: string[] } | null>(null);

  /** 已追问几轮：传给后端写进提示词，让模型知道自己问过几次 */
  const [aiClarifyRound, setAiClarifyRound] = useState(0);

  /** 本轮新增的决策：生成完在侧栏里展示给你看，不对即归入文档形式的“已记入” */
  const [, setAiNewDecisions] = useState<string[]>([]);

  /** 本轮开始前的文档：追问时用它把画布恢复回来（追问不落图） */
  const [docSnapshot, setDocSnapshot] = useState<DbDesignDocument | null>(null);

  /** 决策记录区的展开状态：默认收起——不收起就是一块常驻的字符迹 */
  const [decisionsOpen, setDecisionsOpen] = useState(false);

  /** 决策记录的输入草稿（回车即提交） */
  const [decisionDraft, setDecisionDraft] = useState("");

  const [aiProviders, setAiProviders] = useState<AiProviderLike[]>([]);

  const [aiProviderId, setAiProviderId] = useState("");

  const [aiModelId, setAiModelId] = useState("");

  /**
   * 最新的 `chatMsgs`，供**写盘**时读。
   *
   * 为什么需要 ref：React state 是异步的，一轮生成刚跑完时闭包里的 `chatMsgs`
   * 还是「补上最终气泡之前」的那份。写盘要的是后者。
   * （不能改在 `setChatMsgs` 的 updater 里读——那里面做副作用会被严格模式调用两次。）
   */
  const chatMsgsRef = useRef<ChatMsg[]>(chatMsgs);

  /** 本次运行的 id：发给后端，用于「停止」 */
  const aiRunIdRef = useRef("");

  /** 流式落图时已经排好位的节点（id → 坐标）：同一个节点不会被后来的批次挪动 */
  const aiPlacedRef = useRef<Record<string, { x: number; y: number }>>({});

  const aiProgress = useEventBufferSnapshot(dbdAiProgressBuffer);

  /** 思考的完整文本与是否还在想：供 <ThinkingBlock> 展示。
   *  `text` 从这一版起是**累积全文**（后端以前只给尾部 200 字），展开能看到完整推理链条。
   *  与下面 aiStatusText 用的 aiThinkFinal 的区别：它**不因 done 而变 null** ——
   *  思考完成后那块还在，只是从「思考中」变成可展开的成品，随时能回头看。 */
  const aiThinking = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "reasoning") {
        return { text: e.data?.text ?? "", done: !!e.data?.done };
      }
    }
    return null;
  }, [aiProgress]);

  /** 思考块最终状态：最后一帧 reasoning（done=true 即思考结束，length 为总字数） */
  const aiThinkFinal = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "reasoning") return { length: e.data?.length ?? 0, done: !!e.data?.done };
    }
    return null;
  }, [aiProgress]);

  /** 正文输出的最新帧：运行中是实时预览；跑完后缓冲里最后一帧 = 最终字数 */
  const aiStream = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "stream") return { length: e.data?.length ?? 0, text: e.data?.text ?? "" };
    }
    return null;
  }, [aiProgress]);

  /**
 * 自校验重修进度：最后一帧 repair 事件。
 * 重修期间没有 stream 帧，所以状态条会掉回「思考中」——用它把「正在自动修复 1/2」显示出来，
 * 否则用户只看到卡住不动，不知道系统在替它改错。
 */
const aiRepair = useMemo(() => {
  for (let i = aiProgress.length - 1; i >= 0; i--) {
    const e = aiProgress[i];
    if (e.step === "repair") {
      return { round: e.data?.round ?? 1, max: e.data?.max ?? 2, errors: e.data?.errors ?? 0 };
    }
  }
  return null;
}, [aiProgress]);

/** 状态条 / 胶囊文案：思考中 → 写设计中 → 自动修复中 → 已完成 */
const aiStatusText = aiBusy
  ? aiRepair
    ? t("dbd.aiRepairing", { round: aiRepair.round, max: aiRepair.max })
    : aiStream
      ? t("dbd.aiWritingChars", { count: aiStream.length })
      : aiThinkFinal
        ? t("dbd.aiThinkingChars", { count: aiThinkFinal.length })
        : t("dbd.aiThinking")
    : t("dbd.aiDoneShort");

  /**
   * 切换 AI 模型。
   *
   * **必须持久化**：主模块切换是 `<Activity>` keep-alive，切回时上面那个挂载
   * effect 会重跑一遍「读设置 + 决定用哪个模型」。不写回的话，你刚选好的模型
   * 会在切回时被覆盖回默认 —— 这段代码曾经随 UI 一起被删掉过，后果正是
   * 「选不了模型、而且选的也记不住」。
   *
   * 载荷字段名由后端按名读，不能改（见 modelChoice.modelChoiceSettings）。
   */
  const onAiModelSelect = useCallback((providerId: string, modelId: string) => {
    setAiProviderId(providerId);
    setAiModelId(modelId);
    void invoke("dbd_save_settings", {
      settings: modelChoiceSettings({ providerId, modelId }),
    }).catch(() => {});
  }, []);

  /**
   * AI 生成表结构：后端直接返回一份设计文档，**不落盘** —— 载入画布后由用户自己决定保存。
   * 生成结果没有坐标（AI 不给），所以载入后立刻自动布局 + 适应视野，
   * 否则 100 张表全叠在左上角。
   *
   * **边生成边落图**：后端流式回传部分文档（`dbd-ai-partial`），这里收到就上画布，
   * 不等整篇到齐。位置**只给没排过的新节点**（已排过的一律不动），
   * 否则每来一张表全体重排一次会满屏乱跳；最终那次仍走完整自动布局。
   */
  /**
   * `overrideText`：追问回答时直接发起下一轮。
   *
   * 不把它写进 aiText 再读：那样需要 setState 后等一个循环才能发，
   * 而且用户会看到输入框先清空再出字——两次变化。
   */
  const runAi = async (overrideText?: string) => {
    const ask = (overrideText ?? aiText).trim();
    // 「读盘中」也要挡：换文档 / 切会话的那一瞬，`sessionId` 与 `aiHistory`
    // 还是上一份的，此时**任何**入口发出去都是「新表 + 旧对话」。
    //
    // 闸门放在这里而不是逐个给按钮加 `disabled` —— 发起一轮的入口有三个
    // （发送按钮、追问卡的选项、追问卡的自由输入），UI 上漏一个就绕过了。
    // 只禁发送按钮是不够的：追问卡那会儿还亮着（旧会话的气泡还没被替换）。
    if (!doc || !ask || chatLoading) return;
    setAiBusy(true);
    setAiAborted(false);
    setAiErr("");
    // 开一轮：用户气泡 + 助手占位气泡（进度先去那里【）
    setChatMsgs((m) => startTurn(m, ask));
    // 追问不落图：流式期间后端会阻止推部分文档，但一旦模型先写了几张表才转问，
    // 画布上就会留下半套。快照用来把它撤回去。
    setDocSnapshot(doc);
    dbdAiProgressBuffer.clear();
    dbdAiPartialBuffer.clear();
    // 运行开始的基线快照：画布有表时 AI 基于它修改（后端提示词带它作基线）。
    // 1) 现有表坐标预填进 aiPlacedRef —— 流式落图时它们原地不动；
    // 2) 流式期间「AI 还没重新吐出的表」保留在画布（见 applyPartial 合并）；
    // 3) 收尾时现有表保持手动摆过的位置，只有新表走自动布局。
    const baseDoc = doc.nodes.length > 0 ? doc : null;
    const posSeed: Record<string, { x: number; y: number }> = {};
    for (const n of baseDoc?.nodes ?? []) posSeed[n.id] = { x: n.x ?? 0, y: n.y ?? 0 };
    aiPlacedRef.current = posSeed;
    const runId = crypto.randomUUID();
    aiRunIdRef.current = runId;

    const applyPartial = (partial: DbDesignDocument) => {
      const placed = aiPlacedRef.current;
      let slot = Object.keys(placed).length;
      const laid = partial.nodes.map((n) => {
        const known = placed[n.id];
        if (known) return { ...n, x: known.x, y: known.y };
        const p = {
          x: 40 + (slot % AI_STREAM_COLS) * 320,
          y: 40 + Math.floor(slot / AI_STREAM_COLS) * 300,
        };
        slot += 1;
        placed[n.id] = p;
        return { ...n, ...p };
      });
      if (!baseDoc) {
        // commonFields 要原样带回：partial 是 AI 吐的新文档（只管表结构），
        // 直接采纳会把用户配好的公共字段抹掉 —— 而这是新建场景（画布还没表）唯一的分支。
        applyGeneratedDoc({ ...partial, commonFields: doc.commonFields, nodes: laid });
        return;
      }
      // 修改场景：AI 要重吐全部表，未吐到的先留在画布（按 id、再按名字匹配——
      // 提示词里带了原 id，正常会被原样抄回；漏抄也能靠名字接上），吐到了就顶替。
      const emittedId = new Set(partial.nodes.map((n) => n.id));
      const emittedName = new Set(partial.nodes.map((n) => n.name.trim().toLowerCase()));
      const keptNodes = baseDoc.nodes.filter(
        (n) => !emittedId.has(n.id) && !emittedName.has(n.name.trim().toLowerCase()),
      );
      const emittedRelId = new Set(partial.relations.map((r) => r.id));
      const keptRels = baseDoc.relations.filter((r) => !emittedRelId.has(r.id));
      applyGeneratedDoc({
        ...baseDoc,
        name: partial.name || baseDoc.name,
        nodes: [...laid, ...keptNodes],
        relations: [...partial.relations, ...keptRels],
      });
    };

    // 订阅缓冲（模块级单例，面板切走再回来也不会漏）；退订放在 finally
    const unsubPartial = dbdAiPartialBuffer.subscribe(() => {
      const items = dbdAiPartialBuffer.snapshot();
      const last = items[items.length - 1];
      // runId 对不上 = 上一轮残留（用户连点两次生成），丢弃
      if (!last?.doc || last.runId !== runId) return;
      const firstBatch = Object.keys(aiPlacedRef.current).length === 0;
      applyPartial(last.doc);
      // 只在第一批落图时适应一次视野，之后让画面自己长，不追着镜头跑
      if (firstBatch) fitCanvas();
    });

    try {
      const outcome = await invoke<AiOutcome>("dbd_ai_generate", {
        input: {
          text: ask,
          // 会话历史（滚动窗口）与追问轮次：这两个让模型知道「刚问过什么」「你不能再问了」
          history: aiHistory,
          clarifyRound: aiClarifyRound,
          name: doc.name || undefined,
          dialect: doc.dialect,
          providerId: aiProviderId || null,
          modelId: aiModelId || null,
          runId,
          // 画布有表就把当前设计带上：AI 基于它修改（输出完整更新后的设计）；
          // 是「新建」还是「修改」由模型看需求判断（系统提示里写明了）。
          //
          // **无条件传整个 doc**（不再按「有没有表」筛）：公共字段配置是**文档级设置**，
          // 哪怕画布上一张表都没有，它也必须跟着走 —— 否则「刚配好公共字段就点 AI 生成」
          // 这条最自然的路径，提示词里恰好没有那句声明，模型于是自己建一遍 created_at，
          // 而导出时又因重名被跳过（表自己的赢），用户完全看不出问题出在哪。
          // 「提示词要不要展示当前设计」由后端按「有没有表」判断
          // （build_user_prompt 里过滤空节点文档），所以这里放开不会让空文档进提示词。
          currentDoc: doc,
        },
      });
      // 追问分支：文档一个字都不动。流式期间后端已阻止推部分文档，
      // 但为了不让「半套设计」留在画布上，这里把本轮开始前的文档写回去。
      if (outcome.kind === "clarify") {
        if (docSnapshot) applyGeneratedDoc(docSnapshot, { resetPositions: false });
        setAiClarify({ question: outcome.question, options: outcome.options });
        setChatMsgs((m) => patchLast(m, { status: "clarify", clarify: { question: outcome.question, options: outcome.options } }));
        setAiHistory((h) =>
          trimHistory([...h, { role: "assistant", text: `【追问】${outcome.question}` }]),
        );
        // 追问轮次 +1：后端据此在提示词里写明「你已经问过 N 轮，不要再问」。
        // 它守的是「一直追问下去」这个最坏情况 —— 用户不答就一直问。
        setAiClarifyRound((n) => n + 1);
        setAiText("");
        return;
      }
      const generated = outcome.document;
      // **决策记录的所有权在前端**：模型每轮都要回吐整份文档，
      // 很可能把你之前写的决策丢了或改写。所以保留原文档的这份，
      // 只把本轮新增的叠上去（同文不重复追加）。
      const mergedDecisions = mergeDecisions(doc.decisions, outcome.decisions);
      // 新表的位置跟随用户当前的布局偏好：否则界面选「核心左右」、AI 生成的新表却摆成放射，
      // 同一幅画里两种形态混着，比一开始就用放射还怪。
      const pos = computeLayout(generated, { fieldRows }, layoutMode);
      const placed: DbDesignDocument = {
        ...generated,
        // **公共字段配置必须从原文档带过来**：AI 返回的是一份全新文档（它只管表结构），
        // 直接采纳的话，用户配好的公共字段会被一次生成抹掉 —— 而这配置只存在于设计文件里，
        // 抹掉就等于白配（还得重新勾一遍）。
        commonFields: doc.commonFields,
        decisions: mergedDecisions,
        nodes: generated.nodes.map((n) => {
          // 现有表（按 id、再按名字匹配）保持手动摆过的位置，新表才用自动布局
          const old =
            baseDoc?.nodes.find((o) => o.id === n.id) ??
            baseDoc?.nodes.find((o) => o.name.trim().toLowerCase() === n.name.trim().toLowerCase());
          if (old) return { ...n, x: old.x ?? 0, y: old.y ?? 0 };
          const p = pos[n.id];
          return p ? { ...n, x: Math.round(p.x), y: Math.round(p.y) } : n;
        }),
      };
      applyGeneratedDoc(placed);
      fitCanvas(true);
      // 成功之后才记历史。assistant 那一轮**只存决策摘要**，不存整份 JSON ——
      // 下一轮模型看的是更新后的文档，旧 JSON 里的表定义还是那些，纯烧 token。
      // 只留最近 3 轮（滚动窗口）。
      setAiNewDecisions(outcome.decisions);
      // 本轮收尾的三个真值**先算出来**，state 与落盘共用同一份。
      //
      // 不能写完 state 再从 `chatMsgsRef` / `aiHistory` / `aiClarifyRound` 里读：
      // React 自动批处理意味着那一刻还没 re-render，读到的全是**上一帧**——
      // 于是每轮成功生成后落盘的末条气泡都卡在 status:"writing"，
      // 重启看着像还在写，且决策摘要与本轮历史全丢。
      const donePatch = {
        status: "done" as const,
        summary: t("dbd.aiDone", { count: placed.nodes.length }),
        thinking: aiThinking?.text,
        chars: aiStream?.length,
        decisions: outcome.decisions,
        patch: outcome.kind === "patch" ? patchCounts(outcome.applied) : undefined,
        truncated: outcome.kind === "design" && outcome.truncated,
      };
      const roundSummary =
        outcome.decisions.length > 0
          ? `已按需求修改设计（${outcome.decisions.length} 项决策）`
          : "已按需求修改设计";
      const finalMsgs = patchLast(chatMsgsRef.current, donePatch);
      const finalHistory = trimHistory([
        ...aiHistory,
        { role: "user" as const, text: ask },
        { role: "assistant" as const, text: roundSummary },
      ]);
      setChatMsgs(finalMsgs);
      setAiHistory(finalHistory);
      setAiClarify(null);
      setAiClarifyRound(0);
      // 侧栏保持展开（你还要接着聊），但输入框清空：它是「你正在写什么」，
      // 留着上一句会让人以为还没发出去。历史里有上一轮，问「接着改」也够用了。
      // 自校验结果要说清楚：用户必须知道这份设计**现在能不能用**。
      // 这正是 Q-0410 那个死锁的教训 —— 「满屏报错且导不出来」而界面一声不吭。
      if (outcome.kind === "patch") {
        const a = outcome.applied;
        const parts: string[] = [];
        if (a.addedTables.length) parts.push(t("dbd.aiPatchAdded", { count: a.addedTables.length }));
        if (a.updatedTables.length) parts.push(t("dbd.aiPatchUpdated", { count: a.updatedTables.length }));
        if (a.removedTables.length) parts.push(t("dbd.aiPatchRemoved", { count: a.removedTables.length }));
        toast(parts.length ? parts.join("\uff0c") : t("dbd.aiPatchNoop"), "ok");
        if (a.droppedRelations.length) {
          toast(t("dbd.aiPatchDropped", { count: a.droppedRelations.length }), "err");
        }
      }
      // 产出被截断：这里必须强调“不完整”——用户看到了表，
      // 但不知道关联与决策记录被截尾了。不说他会以为完整。
      if (outcome.kind === "design" && outcome.truncated) {
        toast(t("dbd.aiTruncated", { count: placed.nodes.length }), "err");
      }
      if (outcome.repair.ok) {
        if (outcome.repair.rounds > 0) {
          toast(t("dbd.aiRepaired", { rounds: outcome.repair.rounds }), "ok");
        }
      } else {
        // 没修好也照样落图（否则这轮钱白花、你也看不到设计），但明确告知剩余问题
        toast(
          t("dbd.aiRepairFailed", { count: outcome.repair.remainingErrors.length }),
          "err",
        );
      }
      toast(t("dbd.aiDone", { count: placed.nodes.length }), "ok");
      // 会话落盘**只在这一处**，刻意不放在 finally：
      // 前序设计定了「生成失败 / 你点停止 → 没有历史，这一轮没成功就不该留痕」，
      // 放 finally 会把失败那轮也写进去。
      //
      // 写的是上面算出的 finalMsgs / finalHistory（与 state 同一份真值），
      // 不是 chatMsgsRef —— 批处理下 ref 还是上一帧。
      void commitChat({
        msgs: finalMsgs,
        history: finalHistory,
        clarifyRound: 0,
      });
    } catch (e) {
      const msg = String(e);
      // 主动停止不算失败：只闪一句提示，不要把「已取消」当错误堆在界面上
      if (msg.includes("已取消")) {
        setAiAborted(true);
        setChatMsgs((m) => patchLast(m, { status: "done", error: t("dbd.aiCancelled") }));
        toast(t("dbd.aiCancelled"), "ok");
      } else {
        setAiErr(msg);
        setChatMsgs((m) => patchLast(m, { status: "error", error: msg, thinking: aiThinking?.text }));
      }
    } finally {
      unsubPartial();
      aiRunIdRef.current = "";
      setAiBusy(false);
    }
  };

  /** 停止生成：后端靠 run_id 在全局集合里对上号（前端不等待结果，界面立刻恢复） */
  const cancelAi = async () => {
    const runId = aiRunIdRef.current;
    if (!runId) return;
    try {
      await invoke("dbd_ai_cancel", { runId });
    } catch (e) {
      console.error("停止 AI 失败", e);
    }
  };

  useEffect(() => {
    chatMsgsRef.current = chatMsgs;
  }, [chatMsgs]);

  /** 进度变化时把当轮气泡也更新一下：思考中→正在写、字数不断增长。
      不跟的话气泡会一直卡在「思考中」，用户以为卡死了。*/
  useEffect(() => {
    if (!aiBusy) return;
    setChatMsgs((m) =>
      patchLast(m, {
        status: aiStream ? "writing" : "thinking",
        thinking: aiThinking?.text,
        chars: aiStream?.length,
      }),
    );
  }, [aiBusy, aiThinking, aiStream]);

  // 供应商 / 模型列表来自 AI 模块的现有配置（与思维导图、API 模块同一份）
  useEffect(() => {
    // 记下「上次用的模型」，与设置读**并行**发起：串行会让空态闪一下
    const remembered = invoke<{ lastProviderId?: string; lastModelId?: string }>(
      "dbd_get_settings",
    ).catch(() => null);
    invoke<any>("get_ai_config")
      .then(async (cfg) => {
        const list: AiProviderLike[] = (cfg?.providers ?? []).filter(
          (p: AiProviderLike) => p.api_key && p.openai_url,
        );
        setAiProviders(list);
        const saved = await remembered;
        // 沿用规则（逐级回落 + 先验证还存不存在）挪到 modelChoice.ts：
        // 它连同「切换后写回 dbd_save_settings」是一条完整的契约，拆散在两个
        // effect 里时，其中一半很容易被面板重写顺手带走又没人发现。
        const choice = resolveModelChoice({ providers: list, remembered: saved });
        if (!choice) return;
        setAiProviderId(choice.providerId);
        setAiModelId(choice.modelId);
      })
      .catch(() => {});
  }, []);
  return {
    aiText,
    setAiText,
    aiBusy,
    aiHistory,
    setAiHistory,
    chatMsgs,
    setChatMsgs,
    setAiClarify,
    aiClarifyRound,
    setAiClarifyRound,
    setAiNewDecisions,
    setDocSnapshot,
    decisionsOpen,
    setDecisionsOpen,
    decisionDraft,
    setDecisionDraft,
    aiProviders,
    aiProviderId,
    aiModelId,
    chatMsgsRef,
    aiStatusText,
    onAiModelSelect,
    runAi,
    cancelAi,
  };
}
