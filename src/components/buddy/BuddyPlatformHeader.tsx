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
      <div className="flex items-center gap-1 ml-3 bg-black/30 rounded-ctl border border-white/10 p-0.5 shrink-0">
        {PLATFORMS.map((p) => (
          <button
            key={p.id}
            onClick={() => setPlatform(p.id)}
            aria-busy={platform === p.id && platformLoading}
            className={`px-2.5 py-1 rounded-md text-caption whitespace-nowrap shrink-0 transition cursor-pointer ${
              platform === p.id
                ? "bg-[var(--module-accent)]/25 text-white font-semibold"
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
      <div className="flex items-center gap-1 ml-3 bg-black/30 rounded-ctl border border-white/10 p-0.5 shrink-0">
        {tabs.map((tabItem) => (
          <button
            key={tabItem.id}
            onClick={() => setTab(tabItem.id)}
            className={`px-2.5 py-1 rounded-md text-caption whitespace-nowrap shrink-0 transition cursor-pointer flex items-center gap-1 ${
              tab === tabItem.id
                ? "bg-[var(--module-accent)]/25 text-white font-semibold"
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
