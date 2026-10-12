// 画布节点卡片：仿思维导图 —— **默认折叠**，只显示「表名 + 注释 + 参与关联的字段」，
// 鼠标移上来（或选中）才展开完整字段表。一屏几十张表时，折叠态才读得下去。
//
// 连线锚点（PowerDesigner 式）：
//   · 左侧一个「整表」入口 —— 拖它到另一张表 = 复制本表**全部主键**字段过去并建外键（复合主键用这个）；
//   · 每一行字段左右各一个锚点 —— 右侧拖出 = 用该字段建关联；左侧是连线的落点，
//     所以「A 的主键 → B 的外键」能一一对应，复合主键不会全挤在表头一个点上。
//   · 若源字段是主键且目标表没有同名字段，自动把字段复制过去（PowerDesigner 的行为）。
import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { Database, Eye, KeyRound } from "lucide-react";

import { typeLabel, type DbDesignNode, type FieldRowMode } from "./types";

/** 注释列最多显示几个字符（超出的截断，完整内容在 title 里） */
const COMMENT_MAX = 8;

export interface DesignNodeData extends Record<string, unknown> {
  node: DbDesignNode;
  /** 折叠态显示的字段（参与关联的那些） */
  relationFields: string[];
  /** 展开态：由面板按「节点里显示 = 全部字段」推导（不再有独立的常驻展开开关）。
   *  刻意不做 hover / 选中触发展开 —— 移开鼠标又缩回去，读字段时来回挪很烦。 */
  expanded: boolean;
  selected: boolean;
  /** 折叠态显示哪些字段：只主键+外键，还是全部 */
  fieldRows: FieldRowMode;
  /** 外键字段名（B 侧「多」端那一列）—— fieldRows="keys" 时用它筛 */
  fkFields: string[];
  /**
   * 与「当前选中表」存在关联（直接引用或被引用）。
   * 选中某张表时用它把关联表点亮、其余压暗 —— 表一多，静态图里
   * 「哪几张跟我有关」只能靠眼睛一根根线扫。
   */
  related: boolean;
}

export const HANDLE_TABLE = "table";
export const handleField = (field: string) => `f:${field}`;
/**
 * 左侧（被引用侧）字段锚点的 id。
 *
 * 为什么要和右侧分开命名：为了让**同一个锚点既能接、也能拖**（用户要求左右双向等价），
 * React Flow 需要 `connectionMode="loose"`；而 loose 模式下锚点不再按声明类型区分，
 * id 必须左右各一个，否则同名字段两个锚点会被当成同一个。
 * 面板里靠这个前缀判断「拖线是从哪一侧开始的」，见 DbDesignerPanel.onConnect。
 */
export const handleFieldIn = (field: string) => `b:${field}`;
export const HANDLE_TABLE_IN = "table-in";
export const handleAllPk = "pk-all";

function DesignNodeCardInner({ data }: NodeProps) {
  const d = data as unknown as DesignNodeData;
  const { node, relationFields, expanded, selected, fieldRows, fkFields, related, dimmed } = d;
  const isView = node.kind === "view";
  const fields = node.table?.fields ?? [];
  /** 表配色：图标 / 表名 / 表头底 / 整卡底 / 左侧色条都用它；没设置就回退模块强调色。
   *  rawColor 单独留一份原始 hex（仅用户显式选色时非空）：背景要按透明度混色，
   *  CSS 变量拼不了 alpha 值，所以两者分开。 */
  const rawColor = /^#[0-9a-f]{6}$/i.test(node.color ?? "") ? (node.color as string) : "";
  const tint = rawColor || "var(--module-accent)";
  /**
   * 高亮色 = **这张表自己的颜色**（同系列自动配色或用户选的色）；没设色才回退
   * （选中→主色，关联→青）。
   *
   * 早先固定用主色/青，理由是「怕深色表被高亮后反而更暗」。但那会**色相打架**：
   * 红的表被套一圈青霓虹，两个色相对着干，看着就是错的。改为跟随表色后，
   * 高亮环与卡片、左侧色带、它的连线是同一个颜色 —— 一整套语言自洽。
   *
   * 档位因此改用**强度**区分（描边粗细 + 辉光层数 + 表头浓度），不再靠色相；
   * 选中档再把表色往白里提 12%，暗色表也能看清环。
   *
   * 三档里的 `dimmed` 由面板侧算好后传进来，**不能在卡片里自己判断**：
   * 「没选中任何表」也是一种正常状态，不该被当成「这张表跟谁都没关系」而整片压暗。
   */
  const HL_FALLBACK_SELECT = "var(--module-accent, var(--vex-fallback-primary))";
  const HL_FALLBACK_RELATED = "var(--cyan, var(--vex-cyan, var(--vex-fallback-cyan)))";
  /** 本卡片当前的高亮色（未高亮时为空串） */
  const hlColor = selected
    ? rawColor
      ? `color-mix(in srgb, ${rawColor} 88%, white 12%)`
      : HL_FALLBACK_SELECT
    : related
      ? rawColor || HL_FALLBACK_RELATED
      : "";
  /** 霓虹环：贴边一层实描边 + 两三层渐远外辉光；选中更粗、辉光更多层 */
  const ring = selected
    ? `0 0 0 2px ${hlColor}, 0 0 6px color-mix(in srgb, ${hlColor} 60%, transparent), 0 0 18px color-mix(in srgb, ${hlColor} 42%, transparent), 0 0 40px color-mix(in srgb, ${hlColor} 22%, transparent)`
    : related
      ? `0 0 0 1px ${hlColor}, 0 0 8px color-mix(in srgb, ${hlColor} 48%, transparent), 0 0 26px color-mix(in srgb, ${hlColor} 26%, transparent)`
      : "";
  /** 文字霓虹辉光：高亮档才有，选中更亮 */
  const textGlow = selected
    ? `0 0 6px color-mix(in srgb, ${hlColor} 90%, transparent), 0 0 16px color-mix(in srgb, ${hlColor} 55%, transparent)`
    : related
      ? `0 0 8px color-mix(in srgb, ${hlColor} 60%, transparent)`
      : "";
  /**
   * 显示哪些字段行：
   *   keys —— 只主键 + 外键 + 关联字段（「这张表靠什么关联」一眼看到，杂字段不占地方）
   *   all  —— 全部字段，视图卡片同时展开 SQL 全文
   */
  const shown = expanded
    ? fields
    : fields.filter((f) =>
        fieldRows === "all"
          ? true
          : f.pk || fkFields.includes(f.name) || relationFields.includes(f.name),
      );

  return (
    <div
      className={`group relative w-[288px] overflow-hidden rounded-card border bg-surface-panel transition-all ${
        ring ? "" : "border-white/10 shadow-lg"
      } ${dimmed ? "opacity-35" : "opacity-100"}`}
      style={{
        // 选色后：整卡底色混入 6% 的表色，让颜色落在「卡身」上，而不只是左侧一条色带
        ...(rawColor ? { background: `color-mix(in srgb, ${rawColor} 6%, var(--color-surface-panel))` } : null),
        // 高亮：描边 + 霓虹环（内联写，压过原本的 shadow-lg）；颜色跟随表色（见 hlColor 注释）
        ...(ring ? { borderColor: hlColor, boxShadow: ring } : null),
      }}
    >
      {/* 表配色：左侧一条竖色带，一眼分出这张表属于哪一组（不用读标签文字） */}
      <div className="absolute inset-y-0 left-0 w-[3px]" style={{ background: tint }} />
      {/* 整表入口：手动把关系连到「表」而不是具体某列时落这里。
          自动生成的连线一律落在具体字段行上（复合主键才不会全挤在表头）。 */}
      <Handle
        type="target"
        position={Position.Left}
        id={HANDLE_TABLE_IN}
        isConnectable
        title="被引用的入口：别的表拖线到这里（整表级关联）"
        // 显式指定 top：Handle 默认落在节点垂直中点，字段行锚点各占一行，
        // 不锁到表头那一行就会跟第一行的字段锚点叠成重影（用户截图里就是这个现象）。
        style={{ top: 16 }}
        className="!h-3 !w-3 !rounded-full !border-2 !border-sky-400 !bg-surface-panel opacity-30 transition group-hover:opacity-70 group-hover:opacity-100"
      />

      <div
        className="relative flex items-center gap-1.5 border-b border-white/10 px-2.5 py-1.5"
        // 表头底：高亮时用同一个高亮色（选中最浓，是「点亮」最显眼的一块）；
        // 未高亮时混入 16% 表色（表的身份），没有表色就用极淡的白
        style={{
          background: selected
            ? `color-mix(in srgb, ${hlColor} 26%, transparent)`
            : related
              ? `color-mix(in srgb, ${hlColor} 14%, transparent)`
              : rawColor
                ? `color-mix(in srgb, ${rawColor} 16%, transparent)`
                : "rgb(255 255 255 / 0.04)",
        }}
      >
        {isView ? (
          <Eye className="h-3.5 w-3.5 flex-shrink-0 text-sky-400" />
        ) : (
          <Database
            className="h-3.5 w-3.5 flex-shrink-0"
            // 图标保持表色（身份），高亮时再加一层同色辉光
            style={{
              color: tint,
              ...(ring ? { filter: `drop-shadow(0 0 4px ${hlColor})` } : null),
            }}
          />
        )}
        <span
          className="min-w-0 flex-1 truncate text-body font-semibold text-slate-100"
          // 表名：高亮时叠霓虹辉光（字本身也更亮），选色时字色仍保持表色
          style={{
            ...(selected && !rawColor ? { color: "#fdeff7" } : null),
            ...(rawColor ? { color: rawColor } : null),
            ...(textGlow ? { textShadow: textGlow } : null),
          }}
        >
          {node.name}
        </span>
        <span className="flex-shrink-0 text-micro text-slate-600">{isView ? "view" : `${fields.length}`}</span>
        {/* 整表锚点：拖到另一张表 = 复制**全部**主键字段并建外键（复合主键走这个）。
            做得比字段锚点大一点：金色小点在深色卡片上太难点中，而它是复合主键唯一的入口。 */}
        {!isView && fields.some((f) => f.pk) ? (
          <Handle
            type="source"
            position={Position.Right}
            id={handleAllPk}
            isConnectable
            title="拖到另一张表：复制本表全部主键并建立关联（复合主键用这个）"
            style={{ top: 16 }}
            className="!h-3.5 !w-3.5 !border-2 !border-amber-300 !bg-amber-400 opacity-30 transition group-hover:opacity-70 group-hover:opacity-100 hover:!bg-amber-300"
          />
        ) : null}
      </div>

      {node.comment ? (
        <div className="truncate px-2.5 pt-1.5 text-micro text-slate-500" title={node.comment}>
          {node.comment}
        </div>
      ) : null}

      {isView ? (
        <div className="px-2.5 py-2 text-micro text-slate-600">
          {expanded ? <span className="line-clamp-3 font-mono">{node.view?.sql}</span> : "SQL 视图"}
        </div>
      ) : (
        <div className="py-1">
          {shown.length === 0 ? (
            <div className="px-2.5 py-1 text-micro text-slate-600">
              {fields.length === 0 ? "还没有字段" : "（无关联字段）"}
            </div>
          ) : (
            shown.map((f) => (
              /* 三列：字段名 / 类型 / 注释。注释超长截断（完整内容在 title），
                 否则一个宽表能把卡片撑到没法看。
                 group/row：悬停某一行时只有这一行的锚点亮起（一屏几十行时不会全在闪）。 */
              <div key={f.name} className="group/row relative flex items-center gap-1.5 px-2.5 py-0.5 text-micro hover:bg-white/[0.04]">
                {/* 左：被引用侧（冷色空心）。既接别人的线，也能从这里拖出去 —— 两侧等价。 */}
                <Handle
                  type="target"
                  position={Position.Left}
                  id={handleFieldIn(f.name)}
                  isConnectable
                  title="被引用：别的表拖线到这一列（也可反向从这边拖出，结果一样）"
                  className="!h-2.5 !w-2.5 !rounded-full !border-2 !border-sky-400/90 !bg-surface-panel opacity-25 transition group-hover:opacity-70 group-hover/row:!opacity-100"
                />
                <span className="min-w-0 flex-1 truncate text-slate-300">
                  {f.pk ? (
                    <span className="mr-0.5 inline-flex items-center text-amber-400">
                      <KeyRound className="h-2.5 w-2.5" />
                    </span>
                  ) : null}
                  {f.name}
                </span>
                <span className="w-[68px] flex-shrink-0 truncate font-mono text-slate-600">
                  {typeLabel(f.type)}
                </span>
                <span className="w-[64px] flex-shrink-0 truncate text-slate-500" title={f.comment || undefined}>
                  {f.comment ? (f.comment.length > COMMENT_MAX ? `${f.comment.slice(0, COMMENT_MAX)}…` : f.comment) : ""}
                </span>
                {/* 右：引用别人（暖色实心）。既能拖出去，也能接别人的线 —— 两侧等价。 */}
                <Handle
                  type="source"
                  position={Position.Right}
                  id={handleField(f.name)}
                  isConnectable
                  title="引用别人：从这一列拖出（也可把别的表拖线到这边，结果一样）"
                  className="!h-2.5 !w-2.5 !rounded-full !border-2 !border-amber-400 !bg-amber-400 opacity-25 transition group-hover:opacity-70 group-hover/row:!opacity-100"
                />
              </div>
            ))
          )}
          {!expanded && fields.length > shown.length ? (
            <div className="px-2.5 pt-0.5 text-micro text-slate-700">
              还有 {fields.length - shown.length} 个字段…
            </div>
          ) : null}
        </div>
      )}
    </div>
  );
}

// memo：拖拽时 React Flow 会频繁重渲染，卡片不 memo 会整画布闪。
export default memo(DesignNodeCardInner);