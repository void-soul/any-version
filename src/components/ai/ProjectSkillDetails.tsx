import { useTranslation } from 'react-i18next';
import { X, Unlink } from 'lucide-react';
import { type ProjectSkillEntry, type SkillProject } from "./SkillManager.model";

interface ProjectSkillDetailsProps {
  selProject: string | null;
  projSkills: Record<string, ProjectSkillEntry[]>;
  projects: SkillProject[];
  cleanupBroken: (pid: string) => Promise<void>;
  projBusy: string | null;
  openPreview: (e: { skillId: string; name: string; }) => Promise<void>;
  removeSkillFromDir: (entry: ProjectSkillEntry, dirName: string) => void;
  projMsg: { id: string; msg: string; ok: boolean; } | null;
}

/** Pure ProjectSkillDetails view; its owning panel retains side effects and state. */
export function ProjectSkillDetails({ selProject, projSkills, projects, cleanupBroken, projBusy, openPreview, removeSkillFromDir, projMsg }: ProjectSkillDetailsProps) {
  const { t } = useTranslation();
  return (
    <div className="min-w-0">
      {!selProject ? (
        <div className="rounded-card bg-white/[0.02] border border-white/10 text-center py-16 text-slate-500 text-sm">
          {t("skillmgr.projPickOne")}
        </div>
      ) : (
        (() => {
          const list = projSkills[selProject] || [];
          const broken = list.filter((e) => e.state === 'broken');
          const alive = list.filter((e) => e.state !== 'broken');
          const viewing = projects.find((p) => p.id === selProject);
          return (
            <div className="rounded-card bg-white/[0.02] border border-white/10 p-3 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <span className="text-body font-bold text-slate-300 truncate">
                  {t("skillmgr.projInstalledTitle")}
                  <span className="ml-1.5 text-micro text-slate-500 font-normal">{viewing?.path}</span>
                </span>
                {/* 原先这里有「批量删除 (N)」按钮 + 每行一个勾选框。
                    已按用户要求删除：改成每一行直接列出装了哪些目录、
                    点目录标签上的 × 逐个去掉。理由是「全卸」与「都不卸」
                    两个极端之间没有中间态，而一个技能同时装在
                    .agents 与 .codebuddy 是常态。 */}
              </div>

              {broken.length > 0 && (
                <div className="p-2 rounded-ctl bg-amber-500/10 text-amber-400 text-tiny flex items-center justify-between gap-2">
                  <span className="min-w-0">
                    {t("skillmgr.projBrokenHint", { names: broken.map((b) => b.skillId).join('、') })}
                  </span>
                  <button
                    onClick={() => void cleanupBroken(selProject)}
                    disabled={projBusy === selProject}
                    className="px-2 py-0.5 rounded bg-amber-500/20 hover:bg-amber-500/30 cursor-pointer disabled:opacity-50 flex-shrink-0"
                  >
                    <Unlink className="w-3 h-3" /> {t("skillmgr.projBrokenClean")}
                  </button>
                </div>
              )}

              {alive.length === 0 ? (
                <div className="text-tiny text-slate-500 py-6 text-center">{t("skillmgr.projInstalledEmpty")}</div>
              ) : (
                <div className="space-y-1">
                  {alive.map((e) => {
                    return (
                      <div
                        key={e.skillId}
                        className="flex items-center gap-2 px-2 py-1.5 rounded-ctl bg-white/[0.03]"
                      >
                        <button
                          onClick={() => void openPreview(e)}
                          className="text-tiny font-semibold text-slate-200 hover:text-[var(--module-accent)] underline decoration-dotted underline-offset-2 truncate cursor-pointer"
                          title={t("skillmgr.projViewMd")}
                        >
                          {e.name || e.skillId}
                        </button>
                        <span
                          className={`px-1.5 py-0.5 rounded text-micro font-semibold flex-shrink-0 ${
                            e.state === 'linked'
                              ? 'bg-violet-500/15 text-violet-300'
                              : 'bg-sky-500/15 text-sky-300'
                          }`}
                        >
                          {e.state === 'linked' ? t("skillmgr.projFromLibrary") : t("skillmgr.projFromProject")}
                        </span>
                        {/* 装在哪些目录：**逐个可删**。
                            一个技能同时装在 .agents 与 .codebuddy 是常态，
                            而「全卸」与「都不卸」之间没有中间态 —— 所以每个
                            目录做成一个带 × 的标签，点哪个就去哪个。
                            删掉最后一个目录，这一行自然消失（列表是扫出来的）。 */}
                        {e.dirs && e.dirs.length > 0 && (
                          <span className="flex items-center gap-1 flex-shrink-0">
                            {e.dirs.map((d) => (
                              <span
                                key={d}
                                className="inline-flex items-center gap-0.5 rounded text-micro bg-white/5 text-slate-400 font-mono pl-1 pr-0.5 py-0.5"
                                title={t("skillmgr.installedInDirs")}
                              >
                                {d}
                                <button
                                  type="button"
                                  onClick={() => removeSkillFromDir(e, d)}
                                  disabled={projBusy === selProject}
                                  title={t("skillmgr.projRemoveDirHint", { dir: d })}
                                  className="nodrag shrink-0 rounded px-0.5 text-slate-500 hover:bg-red-500/25 hover:text-red-300 disabled:opacity-40 cursor-pointer"
                                >
                                  <X className="w-2.5 h-2.5" />
                                </button>
                              </span>
                            ))}
                          </span>
                        )}
                        {/* 有一个链接还有效就仍算已安装，但失效的那些要能被看见 */}
                        {(e.staleLinks ?? 0) > 0 && (
                          <span
                            className="px-1.5 py-0.5 rounded text-micro bg-amber-500/20 text-amber-400 flex-shrink-0"
                            title={t("skillmgr.staleLinksHint", { n: e.staleLinks ?? 0 })}
                          >
                            {t("skillmgr.staleLinks", { n: e.staleLinks ?? 0 })}
                          </span>
                        )}
                        <span className="text-micro text-slate-500 font-mono flex-shrink-0">
                          {t("skillmgr.skillChars", { n: e.skillMdChars })}
                        </span>
                        {e.category && (
                          <span className="px-1.5 py-0.5 rounded text-micro bg-[var(--module-accent-soft)] text-[var(--module-accent)] flex-shrink-0">
                            {e.category}
                          </span>
                        )}
                        {e.tags.map((tag) => (
                          <span key={tag} className="px-1.5 py-0.5 rounded text-micro bg-white/5 text-slate-400 flex-shrink-0">
                            #{tag}
                          </span>
                        ))}
                      </div>
                    );
                  })}
                </div>
              )}

              {projMsg?.id === selProject && (
                <div className={`text-tiny break-all ${projMsg.ok ? 'text-emerald-400' : 'text-red-400'}`}>
                  {projMsg.msg}
                </div>
              )}
            </div>
          );
        })()
      )}
    </div>
  );
}
