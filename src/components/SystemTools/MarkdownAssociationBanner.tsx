import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { X, Check } from "lucide-react";
import { type AssocStatus } from "./MarkdownReader.model";

interface MarkdownAssociationBannerProps {
  toggleAssoc: (register: boolean) => Promise<void>;
  btn: string;
  assocAll: boolean;
  assoc: AssocStatus | null;
}

/** Pure MarkdownAssociationBanner view; its owning panel retains side effects and state. */
export function MarkdownAssociationBanner({ toggleAssoc, btn, assocAll, assoc }: MarkdownAssociationBannerProps) {
  const { t } = useTranslation();
  return (
    <div className="flex-shrink-0 px-4 py-2.5 border-b border-white/5 bg-white/[0.02] space-y-2">
      <div className="text-tiny text-slate-400">
        {t("mdreader.assocDesc")}
      </div>
      <div className="flex items-center gap-2">
        <button onClick={() => void toggleAssoc(true)} className={btn} disabled={assocAll}>
          <Check className="w-3.5 h-3.5" />
          {assocAll ? t("mdreader.registered") : t("mdreader.registerAssoc")}
        </button>
        <button onClick={() => void toggleAssoc(false)} className={btn} disabled={assoc ? !assoc.md && !assoc.markdown : false}>
          <X className="w-3.5 h-3.5" />
          {t("mdreader.unregisterAssoc")}
        </button>
        {assoc && (
          <span className="text-tiny text-slate-500 font-mono truncate">
            .md: {assoc.md ? "✓" : "✗"} · .markdown: {assoc.markdown ? "✓" : "✗"} · {assoc.exePath}
          </span>
        )}
      </div>
    </div>
  );
}
