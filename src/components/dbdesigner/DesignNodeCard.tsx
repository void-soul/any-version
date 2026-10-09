// 画布节点卡片：仿思维导图 —— **默认折叠**，只显示「表名 + 注释 + 参与关联的字段」，
// 鼠标移上来（或选中）才展开完整字段表。一屏几十张表时，折叠态才读得下去。
//
// 连线锚点（PowerDesigner 式）：
//   · 左侧一个「整表」入口 —— 拖它到另一张表 = 复制本表**全部主键**字段过去并建外键（复合主键用这个）；
//   · 每一行字段右侧一个锚点 —— 拖它到另一张表 = 用该字段建关联；
//     若源字段是主键且目标表没有同名字段，自动把字段复制过去（PowerDesigner 的行为）。
//   · 目标表左侧一个入口 —— 所有连线都落在它上面。
import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { Database, Eye, KeyRound } from "lucide-react";

import { typeLabel, type DbDesignNode } from "./types";

export interface DesignNodeData extends Record<string, unknown> {
  node: DbDesignNode;
  /** 折叠态显示的字段（参与关联的那些） */
  relationFields: string[];
  /** 展开态由面板按 hover / 选中控制 */
  expanded: boolean;
  selected: boolean;
}

export const HANDLE_TABLE = "table";
export const handleField = (field: string) => `f:${field}`;
export const handleAllPk = "pk-all";

function DesignNodeCardInner({ data }: NodeProps) {
  const d = data as unknown as DesignNodeData;
  const { node, relationFields, expanded, selected } = d;
  const isView = node.kind === "view";
  const fields = node.table?.fields ?? [];
  const shown = expanded ? fields : fields.filter((f) => relationFields.includes(f.name));

  return (
    <div
      className={`group w-[236px] overflow-hidden rounded-card border bg-surface-panel shadow-lg transition-shadow ${
        selected ? "border-[var(--module-accent)]" : "border-white/10"
      }`}
    >
      {/* 目标入口：所有连线落在这里 */}
      <Handle
        type="target"
        position={Position.Left}
        id={HANDLE_TABLE}
        isConnectable
        className="!h-2.5 !w-2.5 !border-2 !border-surface-panel !bg-slate-500"
      />

      <div className="relative flex items-center gap-1.5 border-b border-white/10 bg-white/[0.04] px-2.5 py-1.5">
        {isView ? (
          <Eye className="h-3.5 w-3.5 flex-shrink-0 text-sky-400" />
        ) : (
          <Database className="h-3.5 w-3.5 flex-shrink-0 text-[var(--module-accent)]" />
        )}
        <span className="min-w-0 flex-1 truncate text-body font-semibold text-slate-100">{node.name}</span>
        <span className="flex-shrink-0 text-micro text-slate-600">{isView ? "view" : `${fields.length}`}</span>
        {/* 整表锚点：拖到另一张表 = 复制全部主键字段并建外键（复合主键走这个） */}
        {!isView && fields.some((f) => f.pk) ? (
          <Handle
            type="source"
            position={Position.Right}
            id={handleAllPk}
            isConnectable
            title="拖到另一张表：复制本表主键并建立关联"
            className="!h-2.5 !w-2.5 !border-2 !border-amber-300 !bg-amber-400/80"
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
              <div key={f.name} className="relative flex items-center gap-1.5 px-2.5 py-0.5 text-micro">
                <span className="min-w-0 flex-1 truncate text-slate-300" title={f.comment || undefined}>
                  {f.pk ? (
                    <span className="mr-0.5 inline-flex items-center text-amber-400">
                      <KeyRound className="h-2.5 w-2.5" />
                    </span>
                  ) : null}
                  {f.name}
                </span>
                <span className="flex-shrink-0 font-mono text-slate-600">{typeLabel(f.type)}</span>
                {/* 字段级锚点：默认隐藏，hover 时出现（避免一屏几十个点太吵） */}
                <Handle
                  type="source"
                  position={Position.Right}
                  id={handleField(f.name)}
                  isConnectable
                  className="!h-2 !w-2 !border-0 !bg-slate-500 opacity-0 transition-opacity group-hover:!opacity-100"
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