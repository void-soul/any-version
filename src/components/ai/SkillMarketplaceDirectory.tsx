import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Store, ExternalLink } from 'lucide-react';
import { MARKET_SOURCES } from "./SkillManager.model";

interface SkillMarketplaceDirectoryProps {

}

/** Pure SkillMarketplaceDirectory view; its owning panel retains side effects and state. */
export function SkillMarketplaceDirectory({  }: SkillMarketplaceDirectoryProps) {
  const { t } = useTranslation();
  return (
    <div>
      <div className="text-body font-semibold text-slate-300 mb-2 flex items-center gap-1.5">
        <Store className="w-3.5 h-3.5 text-[var(--module-accent)]" /> {t("skillmgr.marketSources")}
      </div>
      <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5">
        {MARKET_SOURCES.map((src) => (
          <a
            key={src.url}
            href={src.url}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(e) => { e.preventDefault(); void openUrl(src.url); }}
            className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 hover:border-[var(--module-accent-ring)] transition-all group"
          >
            <div className="flex items-center justify-between">
              <span className="text-body font-semibold text-slate-100">{t(src.name)}</span>
              <ExternalLink className="w-3.5 h-3.5 text-slate-500 group-hover:text-[var(--module-accent)] transition-colors" />
            </div>
            <p className="text-tiny text-slate-400 mt-1">{t(src.desc)}</p>
          </a>
        ))}
      </div>
    </div>
  );
}
