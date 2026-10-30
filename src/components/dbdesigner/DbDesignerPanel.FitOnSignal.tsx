import { useEffect, useRef } from "react";
import { useReactFlow } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
/** 画布内的「适应视野」触发器：signal 每 +1 就重新 fitView 一次。
 *  必须待在 ReactFlowProvider 里面（useReactFlow 依赖它），所以单独拆个小组件。
 *
 *  `ids` 给定时只把这批节点纳入视野：「整理选中」后要定位到刚整理的那几张表，
 *  而不是把整份设计（几十张表）缩到全览 —— 那等于没定位。ids 走 ref 传，
 *  这样 effect 只依赖 signal：ids 与 signal 同一次更新里写入，不会触发多余的适配。 */
export function FitOnSignal({ signal, ids }: { signal: number; ids?: string[] }) {
  const { fitView, getNodes } = useReactFlow();
  const first = useRef(true);
  const idsRef = useRef(ids);
  idsRef.current = ids;
  useEffect(() => {
    if (first.current) {
      first.current = false; // 首挂载交给 <ReactFlow fitView>，别动初始视野
      return;
    }
    if (signal > 0) {
      const want = idsRef.current;
      const nodes =
        want && want.length > 0 ? getNodes().filter((n) => want.includes(n.id)) : undefined;
      // 指定了 ids 但一个都没匹配上（节点刚好被删）→ 退回全图，别让视野卡在空处
      void fitView({
        nodes: nodes && nodes.length > 0 ? nodes : undefined,
        padding: 0.15,
        duration: 260,
      });
    }
  }, [signal, fitView, getNodes]);
  return null;
}
