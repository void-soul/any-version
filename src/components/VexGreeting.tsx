import { useEffect, useState } from "react";
import { greetingAt } from "../utils/brand";

/**
 * 轮换的 vex 打招呼文案。任意界面丢一个即可，让 vex 的形象与语气「活」在更多角落。
 * 复用同一套欢迎语素材（utils/brand.ts），保证全 App 语气一致。
 */
export default function VexGreeting({
  seconds = 8,
  nameColor,
}: {
  /** 换一句话的间隔（秒） */
  seconds?: number;
  /** 「vex」名字的颜色（默认取当前模块主题色 var(--module-accent)） */
  nameColor?: string;
}) {
  const [idx, setIdx] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => setIdx((i) => i + 1), Math.max(2, seconds) * 1000);
    return () => window.clearInterval(t);
  }, [seconds]);

  const color = nameColor ?? "var(--module-accent)";
  return (
    <span className="animate-in fade-in duration-500" key={idx}>
      <span className="font-semibold mr-1" style={{ color }}>vex</span>
      {greetingAt(idx)}
    </span>
  );
}