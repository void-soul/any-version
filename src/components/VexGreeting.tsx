import { useEffect, useState } from "react";
import { greetingAt, timeGreeting } from "../utils/brand";

/**
 * 轮换的 vex 打招呼文案 + 打字机效果。任意界面丢一个即可，让 vex 的形象与语气「活」在更多角落。
 * 默认带按时段开场白（早/午/晚），拟真人感；文案逐字打出后停留，再换下一句。
 */
export default function VexGreeting({
  seconds = 8,
  nameColor,
  time = true,
}: {
  /** 换一句话的间隔（秒） */
  seconds?: number;
  /** 「vex」名字的颜色（默认取当前模块主题色 var(--module-accent)） */
  nameColor?: string;
  /** 是否附加按时段开场白（早上好/晚上好…），默认开 */
  time?: boolean;
}) {
  const [idx, setIdx] = useState(0);
  const [shown, setShown] = useState(0);
  const full = `${time ? timeGreeting() : ""}${time ? " " : ""}${greetingAt(idx)}`;

  // 换句轮换
  useEffect(() => {
    const t = window.setInterval(() => setIdx((i) => i + 1), Math.max(2, seconds) * 1000);
    return () => window.clearInterval(t);
  }, [seconds]);

  // 打字机：逐字打出，打完后停留
  useEffect(() => {
    setShown(0);
    const iv = window.setInterval(() => {
      setShown((s) => {
        if (s >= full.length) {
          window.clearInterval(iv);
          return s;
        }
        return s + 1;
      });
    }, 55);
    return () => window.clearInterval(iv);
  }, [idx, full]);

  const color = nameColor ?? "var(--module-accent)";
  return (
    <span>
      <span className="mr-1 font-semibold" style={{ color }}>vex</span>
      {full.slice(0, shown)}
      {shown < full.length && (
        <span className="ml-px inline-block w-[1ch] animate-pulse text-[var(--module-accent)]">▍</span>
      )}
    </span>
  );
}