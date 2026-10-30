import { useState, useEffect } from "react";
import { Copy, Trash2, Pin, Pencil, Check, Folder } from "lucide-react";
import { useTranslation } from "react-i18next";
import { TOKEN_TYPE_LABELS, formatCode, generateCode, remainingSeconds, type OtpToken } from "./OtpPanel.model";
import { BrandIcon } from "./OtpPanel.BrandIcon";
// 令牌卡片
export function TokenCard({
  token,
  now,

  copied,
  categories,
  onCopy,
  onTogglePin,
  onDelete,
  onEdit,
}: {
  token: OtpToken;
  now: number;
  copied: boolean;
  categories: string[];
  onCopy: () => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onEdit: () => void;
}) {
  const { t } = useTranslation();
  const [code, setCode] = useState("------");
  const [remain, setRemain] = useState(30);

  useEffect(() => {
    let alive = true;
    generateCode(token, now).then((c) => {
      if (alive) setCode(c);
    });
    setRemain(remainingSeconds(token, now));
    return () => {
      alive = false;
    };
  }, [token, now]);

  const isHotp = token.tokenType === "HOTP";
  const progress = isHotp ? 0 : remain / (token.period || 30);

  return (
    <div
      className={`group relative rounded-card border p-3 pb-9 transition cursor-pointer ${
        token.pinned
          ? "bg-amber-500/[0.06] border-amber-500/25"
          : "bg-white/[0.03] border-white/10 hover:bg-white/[0.05] hover:border-white/20"
      }`}
      onClick={onCopy}
      title={t("otp.copyCode")}
    >
      <div className="flex items-start gap-2">
        <BrandIcon token={token} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-title font-semibold text-white truncate">
              {token.issuer || t("otp.unknown")}
            </span>
            {token.pinned && <Pin className="w-3 h-3 text-amber-400 flex-shrink-0" />}
          </div>
          <div className="text-tiny text-slate-500 truncate">{token.account}</div>
          {token.description && (
            <div className="text-micro text-slate-600 truncate mt-0.5">{token.description}</div>
          )}
          {/* 所属分类 */}
          {categories.length > 0 && (
            <div className="flex flex-wrap gap-1 mt-1">
              {categories.map((c, i) => (
                <span
                  key={i}
                  className="inline-flex items-center gap-0.5 text-[8px] px-1 py-0.5 rounded bg-[var(--module-accent)]/10 text-[var(--module-accent)] border border-[var(--module-accent)]/20"
                >
                  <Folder className="w-2 h-2" />{c}
                </span>
              ))}
            </div>
          )}
          {/* 标签 */}
          {token.tags && (
            <div className="flex flex-wrap gap-1 mt-1">
              {token.tags.split(",").filter(Boolean).map((t, i) => (
                <span key={i} className="text-[8px] px-1 py-0.5 rounded bg-white/5 text-slate-400">
                  #{t.trim()}
                </span>
              ))}
            </div>
          )}
        </div>
        <span className="text-micro px-1.5 py-0.5 rounded bg-white/5 text-slate-400 font-mono flex-shrink-0">
          {TOKEN_TYPE_LABELS[token.tokenType] || token.tokenType}
        </span>
      </div>

      <div className="mt-3 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-xl font-mono font-bold text-white tracking-wider select-none">
            {formatCode(code)}
          </span>
          {copied ? (
            <Check className="w-4 h-4 text-emerald-400" />
          ) : (
            <Copy className="w-3.5 h-3.5 text-slate-500" />
          )}
        </div>
        {/* 环形倒计时 */}
        {!isHotp && (
          <svg width="24" height="24" viewBox="0 0 24 24" className="flex-shrink-0">
            <circle cx="12" cy="12" r="9" fill="none" stroke="rgba(255,255,255,0.1)" strokeWidth="2.5" />
            <circle
              cx="12"
              cy="12"
              r="9"
              fill="none"
              stroke="var(--module-accent)"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeDasharray={`${2 * Math.PI * 9}`}
              strokeDashoffset={`${2 * Math.PI * 9 * (1 - progress)}`}
              transform="rotate(-90 12 12)"
              style={{ transition: "stroke-dashoffset 1s linear" }}
            />
            <text x="12" y="15" textAnchor="middle" className="fill-slate-400" style={{ fontSize: 8 }}>
              {remain}
            </text>
          </svg>
        )}
      </div>

      {/* hover 操作（左下角，避免与顶部右侧的类型/标签重叠） */}
      <div className="absolute left-2 bottom-2 hidden group-hover:flex items-center gap-1">
        <button
          onClick={(e) => { e.stopPropagation(); onEdit(); }}
          className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer"
          title={t("otp.edit")}
        >
          <Pencil className="w-3 h-3" />
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); onTogglePin(); }}
          className={`p-1 rounded hover:bg-white/10 cursor-pointer ${token.pinned ? "text-amber-400" : "text-slate-400 hover:text-amber-300"}`}
          title={token.pinned ? t("otp.unpin") : t("otp.pin")}
        >
          <Pin className="w-3 h-3" />
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); onDelete(); }}
          className="p-1 rounded hover:bg-red-500/15 text-slate-400 hover:text-red-400 cursor-pointer"
          title={t("otp.delete")}
        >
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
    </div>
  );
}
