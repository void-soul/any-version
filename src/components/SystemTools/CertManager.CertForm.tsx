import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { DNS_PROVIDER_MAP, NODE_TYPE_LABELS, type Certificate, type Credential, type DeployLink, type DeployNode } from "./CertManager.model";
export function CertForm({
  creds,
  nodes,
  initial,
  onDone,
}: {
  creds: Credential[];
  nodes: DeployNode[];
  initial?: Certificate | null;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [domains, setDomains] = useState(initial?.domains.join(", ") ?? "");
  const [email, setEmail] = useState(initial?.email ?? "");
  const [ca, setCa] = useState(initial?.ca ?? "letsencrypt");
  const [validation, setValidation] = useState(initial?.validation ?? "dns");
  const [httpPort, setHttpPort] = useState(initial?.http_port ?? 80);
  const [dnsProvider, setDnsProvider] = useState(initial?.dns_provider ?? "alidns");
  const [credentialId, setCredentialId] = useState(initial?.credential_id ?? "");
  const [renewBefore, setRenewBefore] = useState(initial?.renew_before_days ?? 30);
  const [links, setLinks] = useState<Record<string, string>>(() => {
    const src = initial?.deploy_links?.length
      ? initial.deploy_links
      : (initial?.deploy_node_ids ?? []).map((node_id) => ({ node_id, target: "" }));
    return Object.fromEntries(src.map((l) => [l.node_id, l.target]));
  });
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    try {
      const domList = domains
        .split(/[,\s]+/)
        .map((d) => d.trim())
        .filter(Boolean);
      // 通配符自动补根域
      const wild = domList.find((d) => d.startsWith("*."));
      if (wild) {
        const root = wild.slice(2);
        if (!domList.includes(root)) domList.push(root);
      }
      if (domList.length === 0) return setErr(t("certmgr.domainRequired"));
      if (validation === "http" && domList.some((d) => d.startsWith("*."))) {
        return setErr(t("certmgr.httpWildcard"));
      }
      if (validation === "dns" && !credentialId) {
        return setErr(t("certmgr.credRequired"));
      }
      const deployLinks: DeployLink[] = nodes
        .filter((n) => links[n.id] !== undefined)
        .map((n) => ({ node_id: n.id, target: links[n.id] || "" }));

      if (initial) {
        await invoke("cert_update", {
          id: initial.id,
          domains: domList,
          email,
          ca,
          dnsProvider: validation === "dns" ? dnsProvider : "",
          credentialId: validation === "dns" ? credentialId : "",
          validation,
          httpPort,
          renewBeforeDays: renewBefore,
        });
        await invoke("cert_update_links", { id: initial.id, deployLinks });
      } else {
        await invoke("cert_create", {
          domains: domList,
          email,
          ca,
          dnsProvider: validation === "dns" ? dnsProvider : "",
          credentialId: validation === "dns" ? credentialId : "",
          validation,
          httpPort,
          renewBeforeDays: renewBefore,
          deployLinks,
        });
      }
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.domainLabel")}
          <input value={domains} onChange={(e) => setDomains(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="*.example.com, example.com" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.emailLabel")}
          <input value={email} onChange={(e) => setEmail(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="admin@example.com" />
        </label>
        <label className="text-caption text-slate-400">
          CA
          <select value={ca} onChange={(e) => setCa(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            <option value="letsencrypt" className="bg-slate-800 text-slate-200">{t("certmgr.letsencrypt")}</option>
            <option value="letsencrypt-staging" className="bg-slate-800 text-slate-200">{t("certmgr.letsencryptStaging")}</option>
          </select>
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.validation")}
          <select value={validation} onChange={(e) => setValidation(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            <option value="dns" className="bg-slate-800 text-slate-200">{t("certmgr.validationDns")}</option>
            <option value="http" className="bg-slate-800 text-slate-200">{t("certmgr.validationHttp")}</option>
            <option value="cname" disabled className="bg-slate-800 text-slate-500">{t("certmgr.validationCname")}</option>
          </select>
        </label>
        {validation === "http" && (
          <label className="text-caption text-slate-400">
            {t("certmgr.httpPort")}
            <input type="number" value={httpPort} onChange={(e) => setHttpPort(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
          </label>
        )}
        {validation === "dns" && (
          <>
            <label className="text-caption text-slate-400">
              {t("certmgr.dnsProvider")}
              <select value={dnsProvider} onChange={(e) => setDnsProvider(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
                {[...DNS_PROVIDER_MAP, { id: "custom", label: t("certmgr.custom") }].map((p) => (
                  <option key={p.id} value={p.id} className="bg-slate-800 text-slate-200">{t(p.label)}</option>
                ))}
              </select>
            </label>
            <label className="text-caption text-slate-400">
              {t("certmgr.credLabel")}
              <select value={credentialId} onChange={(e) => setCredentialId(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
                <option value="">{t("certmgr.select")}</option>
                {creds.map((c) => (
                  <option key={c.id} value={c.id}>{c.name}（{c.type}）</option>
                ))}
              </select>
            </label>
          </>
        )}
        <label className="text-caption text-slate-400">
          {t("certmgr.renewDays")}
          <input type="number" value={renewBefore} onChange={(e) => setRenewBefore(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>

      {validation === "http" && (
        <p className="text-tiny text-amber-400/90 bg-amber-500/5 border border-amber-500/10 rounded-ctl p-2">
          {t("certmgr.httpHint")}
        </p>
      )}

      <div className="text-caption text-slate-400">
        {t("certmgr.deployNodes")}
        <p className="mt-0.5 text-tiny text-slate-500">{t("certmgr.deployLinkHint")}</p>
        <div className="mt-2 space-y-1.5">
          {nodes.map((n) => {
            const checked = links[n.id] !== undefined;
            return (
              <div key={n.id} className="flex items-center gap-2 text-tiny text-slate-300 bg-black/30 rounded px-2 py-1.5">
                <label className="flex items-center gap-1.5 cursor-pointer shrink-0">
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) => {
                      setLinks((v) => {
                        const next = { ...v };
                        if (e.target.checked) next[n.id] = next[n.id] ?? "";
                        else delete next[n.id];
                        return next;
                      });
                    }}
                  />
                  {n.name}
                  <span className="text-micro text-slate-500">{NODE_TYPE_LABELS[n.type] ?? n.type}</span>
                </label>
                {checked && (
                  <input
                    value={links[n.id] ?? ""}
                    onChange={(e) => setLinks((v) => ({ ...v, [n.id]: e.target.value }))}
                    className="ml-auto w-64 bg-black/30 rounded px-2 py-1 text-tiny text-slate-200 outline-none"
                    placeholder={t("certmgr.targetPh")}
                  />
                )}
              </div>
            );
          })}
          {nodes.length === 0 && <span className="text-slate-500">{t("certmgr.noNodesHint")}</span>}
        </div>
      </div>

      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">
          {initial ? t("certmgr.save") : t("certmgr.createSave")}
        </button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}
