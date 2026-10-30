import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { ApiEndpoint } from "./types";
import { BODY_TYPES } from "./types";
import { KvEditor, FormDataEditor, VarInput } from "./panelParts";

interface ApiRequestBodyEditorProps {
  updateDraft: (patch: Partial<ApiEndpoint>) => void;
  draft: ApiEndpoint;
  variables: Record<string, string | number | boolean | null>;
}

/** Pure ApiRequestBodyEditor view; its owning panel retains side effects and state. */
export function ApiRequestBodyEditor({ updateDraft, draft, variables }: ApiRequestBodyEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="flex gap-1 flex-wrap">
        {BODY_TYPES.map((b) => (
          <button
            key={b.value}
            onClick={() => updateDraft({ body_type: b.value })}
            className={`px-2.5 py-1 text-caption rounded-md cursor-pointer ${draft.body_type === b.value ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
          >
            {b.label}
          </button>
        ))}
      </div>
      {draft.body_type === "formdata" && (
        <FormDataEditor items={draft.body_form} onChange={(v) => updateDraft({ body_form: v })} envVars={variables} />
      )}
      {draft.body_type === "form" && (
        <KvEditor items={draft.body_urlencoded} onChange={(v) => updateDraft({ body_urlencoded: v })} envVars={variables} placeholderValue="值" />
      )}
      {draft.body_type === "graphql" && (
        <div className="space-y-2">
          <VarInput
            multiline
            rows={6}
            value={draft.body_graphql_query}
            envVars={variables}
            onChange={(v) => updateDraft({ body_graphql_query: v })}
            placeholder={"query GetUser($id: ID!) {\n  user(id: $id) { id name }\n}"}
            className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body font-mono text-slate-200 focus:outline-none"
          />
          <VarInput
            multiline
            rows={3}
            value={draft.body_graphql_variables}
            envVars={variables}
            onChange={(v) => updateDraft({ body_graphql_variables: v })}
            placeholder='{"id": "{{random:int:1:100}}"}'
            className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body font-mono text-slate-200 focus:outline-none"
          />
        </div>
      )}
      {draft.body_type === "binary" && (
        <div className="space-y-1.5">
          <input
            value={draft.body}
            onChange={(e) => updateDraft({ body: e.target.value })}
            placeholder={t("api.localFilePathPh")}
            className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body text-slate-200"
          />
          <button
            onClick={async () => {
              const f = await openDialog({ multiple: false });
              if (f) updateDraft({ body: String(f) });
            }}
            className="text-caption px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer"
          >
            {t("api.chooseFile")}
          </button>
        </div>
      )}
      {(draft.body_type === "raw" || draft.body_type === "json") && (
        <VarInput
          multiline
          rows={7}
          value={draft.body}
          envVars={variables}
          onChange={(v) => updateDraft({ body: v })}
          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
          placeholder={draft.body_type === "json" ? '{"name": "{{random:string:6}}", "age": {{random:int:18:60}}}' : t("api.rawBodyPh")}
        />
      )}
    </div>
  );
}
