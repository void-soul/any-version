import { useTranslation } from "react-i18next";
import { Settings2 } from "lucide-react";
import type { ApiEndpoint, PresetHeaderSet } from "./types";
import { COMMON_AUTO_HEADERS } from "./types";
import { KvEditor } from "./panelParts";
import type { Dispatch, SetStateAction } from "react";

interface ApiRequestHeadersEditorProps {
  hideCommonHeaders: boolean;
  setHideCommonHeaders: Dispatch<SetStateAction<boolean>>;
  applyPreset: (setId: string) => Promise<void>;
  presetSets: PresetHeaderSet[];
  setPresetModal: Dispatch<SetStateAction<boolean>>;
  draft: ApiEndpoint;
  updateDraft: (patch: Partial<ApiEndpoint>) => void;
  variables: Record<string, string | number | boolean | null>;
}

/** Pure ApiRequestHeadersEditor view; its owning panel retains side effects and state. */
export function ApiRequestHeadersEditor({ hideCommonHeaders, setHideCommonHeaders, applyPreset, presetSets, setPresetModal, draft, updateDraft, variables }: ApiRequestHeadersEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 flex-wrap">
        <label className="flex items-center gap-1.5 text-tiny text-slate-400 cursor-pointer">
          <input type="checkbox" checked={hideCommonHeaders} onChange={(e) => setHideCommonHeaders(e.target.checked)} className="accent-[var(--module-accent)]" />
          {t("api.hideCommonHeaders")}
        </label>
        <select
          value=""
          onChange={(e) => { if (e.target.value) applyPreset(e.target.value); }}
          className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-tiny text-slate-300 cursor-pointer"
        >
          <option value="">{t("api.applyPreset")}</option>
          {presetSets.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
        </select>
        <button onClick={() => setPresetModal(true)} className="flex items-center gap-1 text-tiny px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
          <Settings2 className="w-3 h-3" /> {t("api.managePreset")}
        </button>
      </div>
      <KvEditor
        items={hideCommonHeaders ? draft.headers.filter((h) => !COMMON_AUTO_HEADERS.includes(h.key.toLowerCase())) : draft.headers}
        onChange={(v) => {
          // 隐藏模式下把被隐藏的常见头合并回去
          if (hideCommonHeaders) {
            const hidden = draft.headers.filter((h) => COMMON_AUTO_HEADERS.includes(h.key.toLowerCase()));
            updateDraft({ headers: [...v, ...hidden] });
          } else {
            updateDraft({ headers: v });
          }
        }}
        placeholderKey={t("api.headerPh")}
        envVars={variables}
        placeholderValue={t("api.bearerValuePh", { token: "{{token}}" })}
      />
    </div>
  );
}
