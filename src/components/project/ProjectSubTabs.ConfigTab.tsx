import { useState, useEffect } from "react";
import { theamedAlert, alertError } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import MonacoEditor from "../shared/MonacoEditor";
import { invoke } from "@tauri-apps/api/core";
import { Loader, Wrench, Save, FileText } from "lucide-react";
import type { ProjectStatus, ProjectDef, ServiceStatus } from "./types";
import { globalRegistered } from "./ProjectSubTabs.model";
export function ConfigTab({ project, def, onRefresh }: { project: ProjectStatus; def: ProjectDef | null; onRefresh: () => Promise<void> }) {
  const { t } = useTranslation();
  const [configContent, setConfigContent] = useState<string>("");
  const [loadingConfig, setLoadingConfig] = useState(false);
  const [savingConfig, setSavingConfig] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const svc: ServiceStatus | null = project.service_status ?? null;
  const configPath = svc?.config_file ?? null;
  const port = svc?.port || def?.default_port || null;

  // 加载配置文件内容
  const loadConfigContent = async () => {
    if (!configPath) return;
    setLoadingConfig(true);
    setErrorMessage(null);
    try {
      const content = await invoke<string>("read_service_config", { name: project.id });
      setConfigContent(content);
    } catch (e: any) {
      setErrorMessage(e.toString());
    } finally {
      setLoadingConfig(false);
    }
  };

  useEffect(() => {
    loadConfigContent();
  }, [project.id, configPath]);

  const handleSaveConfig = async () => {
    setSavingConfig(true);
    try {
      await invoke("write_service_config", { name: project.id, content: configContent });
      theamedAlert(t("projsub.configSaved"));
      await onRefresh();
    } catch (e: any) {
      alertError(t("projsub.saveFail", { err: String(e) }));
    } finally {
      setSavingConfig(false);
    }
  };

  // Map project ID to language mode in Monaco
  const getEditorLanguage = (projectId: string): string => {
    if (configPath) {
      const lowerPath = configPath.toLowerCase();
      if (lowerPath.endsWith(".toml")) return "toml";
      if (lowerPath.endsWith(".yaml") || lowerPath.endsWith(".yml")) return "yaml";
      if (lowerPath.endsWith(".json")) return "json";
      if (lowerPath.endsWith(".conf") || lowerPath.endsWith(".nginx")) return "nginx";
    }
    switch (projectId) {
      case "mysql":
      case "redis":
      case "postgresql":
        return "ini";
      case "mongodb":
        return "yaml";
      case "nginx":
        return "nginx";
      case "frps":
      case "frpc":
        return "toml";
      default:
        return "ini";
    }
  };

  const handleEditorDidMount = (_editor: any, monaco: any) => {
    // 1. Register Nginx language if not registered
    const languages = monaco.languages.getLanguages();
    if (!languages.some((lang: any) => lang.id === 'nginx')) {
      monaco.languages.register({ id: 'nginx' });
      monaco.languages.setMonarchTokensProvider('nginx', {
        keywords: [
          'http', 'server', 'location', 'listen', 'server_name', 'root', 'index',
          'proxy_pass', 'proxy_set_header', 'try_files', 'rewrite', 'error_page',
          'ssl_certificate', 'ssl_certificate_key', 'ssl_protocols', 'ssl_ciphers',
          'access_log', 'error_log', 'client_max_body_size', 'keepalive_timeout',
          'gzip', 'gzip_types', 'upstream', 'events', 'worker_processes', 'include',
          'default_type', 'sendfile', 'tcp_nopush', 'tcp_nodelay'
        ],
        tokenizer: {
          root: [
            [/[a-zA-Z_]\w*/, { cases: { '@keywords': 'keyword', '@default': 'identifier' } }],
            [/[{}]/, 'delimiter.bracket'],
            [/[;]/, 'delimiter'],
            [/#.*$/, 'comment'],
            [/"([^"\\]|\\.)*"/, 'string'],
            [/'([^'\\]|\\.)*'/, 'string'],
            [/\d+/, 'number']
          ]
        }
      });
    }

    // 2. Register autocompletions for all configuration files to improve the experience!

    // Nginx autocompletions
    if (!globalRegistered.nginxCompletions) {
      monaco.languages.registerCompletionItemProvider('nginx', {
        provideCompletionItems: (_model: any, _position: any) => {
          const suggestions = [
            { label: 'server', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'server {\n\tlisten ${1:80};\n\tserver_name ${2:localhost};\n\tlocation / {\n\t\troot ${3:html};\n\t\tindex ${4:index.html};\n\t}\n}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Server block' },
            { label: 'location', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'location ${1:/} {\n\t${2:proxy_pass http://localhost:8080;}\n}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Location block' },
            { label: 'listen', kind: monaco.languages.CompletionItemKind.Property, insertText: 'listen ${1:80};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Listen port' },
            { label: 'server_name', kind: monaco.languages.CompletionItemKind.Property, insertText: 'server_name ${1:localhost};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Server name' },
            { label: 'root', kind: monaco.languages.CompletionItemKind.Property, insertText: 'root ${1:html};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Root directory' },
            { label: 'proxy_pass', kind: monaco.languages.CompletionItemKind.Property, insertText: 'proxy_pass ${1:http://localhost:8080};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Proxy pass target' },
            { label: 'gzip', kind: monaco.languages.CompletionItemKind.Property, insertText: 'gzip ${1:on};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Gzip compression' },
            { label: 'client_max_body_size', kind: monaco.languages.CompletionItemKind.Property, insertText: 'client_max_body_size ${1:50m};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max upload size' }
          ];
          return { suggestions };
        }
      });
      globalRegistered.nginxCompletions = true;
    }

    // ini autocompletions (MySQL, Redis, PostgreSQL)
    if (!globalRegistered.iniCompletions) {
      monaco.languages.registerCompletionItemProvider('ini', {
        provideCompletionItems: (model: any, _position: any) => {
          const text = model.getValue();
          const suggestions = [];

          if (text.includes('[mysqld]') || text.includes('character-set-server') || text.includes('default-storage-engine')) {
            // MySQL completions
            suggestions.push(
              { label: 'port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'port = ${1:3306}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'MySQL Port number' },
              { label: 'max_connections', kind: monaco.languages.CompletionItemKind.Property, insertText: 'max_connections = ${1:151}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max client connections' },
              { label: 'character-set-server', kind: monaco.languages.CompletionItemKind.Property, insertText: 'character-set-server = ${1:utf8mb4}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Server default charset' },
              { label: 'default-storage-engine', kind: monaco.languages.CompletionItemKind.Property, insertText: 'default-storage-engine = INNODB', detail: 'Default database engine' },
              { label: 'innodb_buffer_pool_size', kind: monaco.languages.CompletionItemKind.Property, insertText: 'innodb_buffer_pool_size = ${1:256M}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'InnoDB buffer pool size' },
              { label: 'sql_mode', kind: monaco.languages.CompletionItemKind.Property, insertText: 'sql_mode = STRICT_TRANS_TABLES,NO_ENGINE_SUBSTITUTION', detail: 'SQL syntax modes' }
            );
          } else if (text.includes('requirepass') || text.includes('appendonly') || text.includes('maxmemory') || text.includes('redis')) {
            // Redis completions
            suggestions.push(
              { label: 'port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'port ${1:6379}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Redis Port' },
              { label: 'bind', kind: monaco.languages.CompletionItemKind.Property, insertText: 'bind 127.0.0.1', detail: 'IP binding' },
              { label: 'requirepass', kind: monaco.languages.CompletionItemKind.Property, insertText: 'requirepass ${1:password}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Redis password authentication' },
              { label: 'maxmemory', kind: monaco.languages.CompletionItemKind.Property, insertText: 'maxmemory ${1:512mb}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max memory limit' },
              { label: 'maxmemory-policy', kind: monaco.languages.CompletionItemKind.Property, insertText: 'maxmemory-policy volatile-lru', detail: 'Eviction policy' },
              { label: 'appendonly', kind: monaco.languages.CompletionItemKind.Property, insertText: 'appendonly yes', detail: 'AOF logging enablement' }
            );
          } else if (text.includes('shared_buffers') || text.includes('listen_addresses') || text.includes('logging_collector') || text.includes('postgresql')) {
            // PostgreSQL completions
            suggestions.push(
              { label: 'port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'port = ${1:5432}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'PostgreSQL Port' },
              { label: 'listen_addresses', kind: monaco.languages.CompletionItemKind.Property, insertText: 'listen_addresses = \'${1:*}\'', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Listen addresses' },
              { label: 'max_connections', kind: monaco.languages.CompletionItemKind.Property, insertText: 'max_connections = ${1:100}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max DB connections' },
              { label: 'shared_buffers', kind: monaco.languages.CompletionItemKind.Property, insertText: 'shared_buffers = ${1:128MB}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Shared buffer pool size' },
              { label: 'logging_collector', kind: monaco.languages.CompletionItemKind.Property, insertText: 'logging_collector = on', detail: 'Enable log collector' }
            );
          }
          return { suggestions };
        }
      });
      globalRegistered.iniCompletions = true;
    }

    // yaml autocompletions (MongoDB)
    if (!globalRegistered.yamlCompletions) {
      monaco.languages.registerCompletionItemProvider('yaml', {
        provideCompletionItems: (model: any, _position: any) => {
          const text = model.getValue();
          const suggestions = [];
          if (text.includes('dbPath') || text.includes('bindIp') || text.includes('systemLog') || text.includes('mongod')) {
            // MongoDB yaml completions
            suggestions.push(
              { label: 'storage.dbPath', kind: monaco.languages.CompletionItemKind.Property, insertText: 'storage:\n  dbPath: ${1:path}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'DB files path' },
              { label: 'net.port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'net:\n  port: ${1:27017}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'MongoDB listen port' },
              { label: 'net.bindIp', kind: monaco.languages.CompletionItemKind.Property, insertText: '  bindIp: 127.0.0.1', detail: 'IP bind list' },
              { label: 'security.authorization', kind: monaco.languages.CompletionItemKind.Property, insertText: 'security:\n  authorization: enabled', detail: 'Enable MongoDB client auth' }
            );
          }
          return { suggestions };
        }
      });
      globalRegistered.yamlCompletions = true;
    }
  };

  return (
    <div className="space-y-6">
      {/* 1. 运行参数显示 */}
      {port && (
        <div className="glass-panel border border-white/5 rounded-panel p-5 bg-white/2 space-y-4">
          <div className="flex items-center gap-2 border-b border-white/5 pb-3">
            <Wrench className="w-4 h-4 text-[var(--module-accent)]" />
            <h4 className="text-body font-semibold text-white">{t("projsub.runParamsTitle")}</h4>
          </div>

          <div className="flex items-center justify-between p-3 bg-black/20 rounded-card border border-white/5">
            <div>
              <span className="text-title text-slate-400 font-semibold block">{t("projsub.listenPort")}</span>
              <span className="text-caption text-slate-500 mt-0.5">{t("projsub.listenPortDesc")}</span>
            </div>
            <span className="text-slate-300 font-mono text-title font-bold bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] px-3 py-1 rounded-ctl">
              {port}
            </span>
          </div>
        </div>
      )}

      {/* 2. 配置文件可视化编辑 */}
      {configPath && (
        <div className="glass-panel border border-white/5 rounded-panel p-5 bg-white/2 space-y-4">
          <div className="flex items-center justify-between border-b border-white/5 pb-3">
            <div className="flex items-center gap-2">
              <FileText className="w-4 h-4 text-[var(--module-accent)]" />
              <div>
                <h4 className="text-body font-semibold text-white">{t("projsub.visEditTitle")}</h4>
                <p className="text-tiny text-slate-500 mt-0.5 font-mono select-all break-all" title={configPath}>
                  {t("projsub.editingPath", { path: configPath })}
                </p>
              </div>
            </div>
            {!loadingConfig && !errorMessage && (
              <button
                onClick={handleSaveConfig}
                disabled={savingConfig}
                className="px-3 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all"
              >
                {savingConfig ? <Loader className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
                {t("projsub.saveConfig")}
              </button>
            )}
          </div>

          {loadingConfig ? (
            <div className="flex items-center justify-center gap-2 text-body text-slate-400 py-12">
              <Loader className="w-4 h-4 animate-spin text-[var(--module-accent)]" /> {t("projsub.readingConfig")}
            </div>
          ) : errorMessage ? (
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-200 text-body rounded-card space-y-2">
              <p>{t("projsub.loadConfigFail", { err: errorMessage })}</p>
              <button
                onClick={loadConfigContent}
                className="px-2.5 py-1 bg-white/5 hover:bg-white/10 border border-white/5 text-slate-300 rounded text-caption cursor-pointer"
              >
                {t("projsub.retryLoad")}
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <div className="border border-white/5 rounded-card overflow-hidden h-80 bg-[#1e1e1e]">
                <MonacoEditor
                  height="100%"
                  language={getEditorLanguage(project.id)}
                  value={configContent}
                  onChange={(val) => setConfigContent(val || "")}
                  theme="vs-dark"
                  onMount={handleEditorDidMount}
                  options={{
                    minimap: { enabled: false },
                    fontSize: 13,
                    fontFamily: "Fira Code, Courier New, monospace",
                    automaticLayout: true,
                    lineNumbers: "on",
                    scrollBeyondLastLine: false,
                    wordWrap: "on",
                    padding: { top: 12, bottom: 12 }
                  }}
                />
              </div>
              <p className="text-caption text-slate-500">
                {t("projsub.editorHint")}
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
