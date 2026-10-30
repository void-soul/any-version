import { useEffect } from "react";
import { useReactFlow } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
/**
 * 把画布平移到某个节点并居中。
 *
 * 表一多，在画布上「找到那张表」就得拖来拖去 —— 这是导航列表存在的原因。
 * 单独做成信号组件：定位需要 `useReactFlow`，而它只能在 ReactFlowProvider 内调用，
 * 面板本体在 Provider 之外。
 */
export function CenterOnSignal({ signal, nodeId }: { signal: number; nodeId: string | null }) {
  const { setCenter, getNode } = useReactFlow();
  useEffect(() => {
    if (signal <= 0 || !nodeId) return;
    const n = getNode(nodeId);
    if (!n) return;
    const w = n.measured?.width ?? n.width ?? 288;
    const h = n.measured?.height ?? n.height ?? 200;
    void setCenter(n.position.x + w / 2, n.position.y + h / 2, { zoom: 1, duration: 320 });
  }, [signal, nodeId, setCenter, getNode]);
  return null;
}
