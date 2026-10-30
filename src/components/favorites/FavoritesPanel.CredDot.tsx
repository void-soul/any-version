
/**
 * 凭证链接右上角的告警点。
 *
 * 只做「一眼看出有事」，具体原因交给 `title`——因为这里能承载的信息量太小，
 * 把「3 天后过期」塞进一个点里只会让人困惑。
 */
export function CredDot({ level }: { level: "expired" | "soon" | null }) {
  if (!level) return null;
  return (
    <span
      className={`absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full ${
        level === "expired" ? "bg-rose-400" : "bg-amber-400"
      }`}
    />
  );
}
