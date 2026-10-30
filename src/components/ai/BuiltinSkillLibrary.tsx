import { useTranslation } from 'react-i18next';
import { ResultNote } from '../shared/Note';
import { Download, Loader2, Sparkles, RotateCw } from 'lucide-react';
import { type BuiltinSkillView, type SkillEntry } from "./SkillManager.model";

interface BuiltinSkillLibraryProps {
  builtins: BuiltinSkillView[];
  skills: SkillEntry[];
  updateBuiltin: (id: string) => Promise<void>;
  installBuiltin: (id: string) => Promise<void>;
  builtinBusy: string | null;
  builtinResult: { ok: boolean; msg: string; } | null;
}

/** Pure BuiltinSkillLibrary view; its owning panel retains side effects and state. */
export function BuiltinSkillLibrary({ builtins, skills, updateBuiltin, installBuiltin, builtinBusy, builtinResult }: BuiltinSkillLibraryProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-4 max-w-4xl">
      {/* 原先这里有一张「内置技能随 Kira 分发…」的说明块（Note tone=info）。
          已删除：它讲的是「装完进技能库、再由你显式安装给项目」这套流程，
          而页面上「添加技能」按钮与装完后的结果条已经把它说清楚了。
          说明块长期占着弹框顶部，而这里要展示的是技能本身。 */}

      {builtins.length === 0 ? (
        <div className="text-center py-16 text-slate-500 text-sm">{t("skillmgr.builtinEmpty")}</div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-2.5">
          {builtins.map((b) => {
            // 内置技能装进技能库后目录名 === id，据此判断是否已装。
            //（安装时不写 manifest，所以没有别的来源可查）
            const installed = skills.some((s) => s.id === b.id);
            return (
            <div key={b.id} className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 space-y-2">
              <div className="flex items-center gap-2">
                <Sparkles className="w-3.5 h-3.5 text-[var(--module-accent)] flex-shrink-0" />
                <span className="text-body font-semibold text-slate-100 truncate">{b.name}</span>
                {installed && (
                  <span className="shrink-0 px-1.5 py-0.5 rounded text-micro font-semibold bg-emerald-500/15 text-emerald-400">
                    {t("skillmgr.builtinInstalledBadge")}
                  </span>
                )}
                <code className={`${installed ? "ml-auto" : "ml-auto"} text-micro text-slate-600 font-mono truncate`}>{b.id}</code>
              </div>
              <p className="text-tiny text-slate-400 leading-relaxed">{b.description}</p>
              <button
                onClick={() => void (installed ? updateBuiltin(b.id) : installBuiltin(b.id))}
                disabled={builtinBusy !== null}
                className={`w-full px-3 py-2 rounded-ctl disabled:opacity-50 text-body font-semibold text-white flex items-center justify-center gap-2 transition-all cursor-pointer ${
                  installed
                    ? "bg-white/8 hover:bg-white/15 border border-white/15"
                    : "bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)]"
                }`}
              >
                {builtinBusy === b.id
                  ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  : installed
                    ? <RotateCw className="w-3.5 h-3.5" />
                    : <Download className="w-3.5 h-3.5" />}
                {builtinBusy === b.id
                  ? t("skillmgr.builtinInstalling")
                  : installed
                    ? t("skillmgr.builtinUpdate")
                    : t("skillmgr.builtinInstall")}
              </button>
            </div>
            );
          })}
        </div>
      )}

      {builtinResult && <ResultNote ok={builtinResult.ok} message={builtinResult.msg} />}
    </div>
  );
}
