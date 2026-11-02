import type { ReactNode } from "react";
import { Loader2, Users } from "lucide-react";
import { PLATFORMS, type Tab } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface BuddyPlatformHeaderProps {
  setPlatform: Dispatch<SetStateAction<string>>;
  platform: string;
  platformLoading: boolean;
  tabs: { id: Tab; label: string; icon: ReactNode; }[];
  setTab: Dispatch<SetStateAction<Tab>>;
  tab: Tab;
}

/** Pure BuddyPlatformHeader view; its owning panel retains side effects and state. */
export function BuddyPlatformHeader({ setPlatform, platform, platformLoading, tabs, setTab, tab }: BuddyPlatformHeaderProps) {
  return (
    <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5 flex-shrink-0 flex-wrap">
      <Users className="w-4 h-4 text-[var(--module-accent)]" />
      {/* 平台切换 */}
      <div className="flex items-center gap-1 ml-3 bg-surface-inset/50 rounded-ctl border border-white/10 p-0.5 min-w-0 max-w-full overflow-x-auto">
        {PLATFORMS.map((p) => (
          <button
            type="button"
            aria-pressed={platform === p.id}
            key={p.id}
            onClick={() => setPlatform(p.id)}
            aria-busy={platform === p.id && platformLoading}
            className={`ui-btn h-8 px-2.5 whitespace-nowrap shrink-0 ${
              platform === p.id
                ? "ui-btn-primary"
                : "text-slate-400 hover:text-white"
            }`}
          >
            <span className="mr-1">{p.emoji}</span>
            {p.label}
            {platform === p.id && platformLoading && (
              <Loader2 className="inline-block w-3 h-3 ml-1 animate-spin align-[-1px]" />
            )}
          </button>
        ))}
      </div>
      {/* Tab 切换 */}
      <div className="flex items-center gap-1 ml-3 bg-surface-inset/50 rounded-ctl border border-white/10 p-0.5 min-w-0 max-w-full overflow-x-auto">
        {tabs.map((tabItem) => (
          <button
            type="button"
            aria-pressed={tab === tabItem.id}
            key={tabItem.id}
            onClick={() => setTab(tabItem.id)}
            className={`ui-btn h-8 px-2.5 whitespace-nowrap shrink-0 flex items-center gap-1 ${
              tab === tabItem.id
                ? "ui-btn-primary"
                : "text-slate-400 hover:text-white"
            }`}
          >
            {tabItem.icon}
            {tabItem.label}
          </button>
        ))}
      </div>
    </div>
  );
}
