import React, { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FileText,
  Copy,
  Check,
  ChevronDown,
  ChevronRight,
  FolderOpen,
} from "lucide-react";
import { marked } from "marked";

// 配置 marked
marked.setOptions({ breaks: true, gfm: true });

function renderMarkdown(content: string): string {
  try {
    return marked.parse(content) as string;
  } catch {
    return content;
  }
}

interface SkillFile {
  path: string;
  contents: string;
}

interface SkillWithFiles {
  id: string;
  name: string;
  directory: string;
  files: SkillFile[];
}

export default function SkillFileViewer({ skillId, onClose }: { skillId: string; onClose: () => void }) {
  const [skill, setSkill] = useState<SkillWithFiles | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());
  const [copiedFile, setCopiedFile] = useState<string | null>(null);

  useEffect(() => {
    loadSkillFiles();
  }, [skillId]);

  const loadSkillFiles = async () => {
    setLoading(true);
    try {
      const [name, files] = await invoke<[string, SkillFile[]]>("get_skill_files", { skillId });
      setSkill({ id: skillId, name, directory: "", files });
      // Auto-expand SKILL.md
      if (files.some(f => f.path === "SKILL.md")) {
        setExpandedFiles(new Set(["SKILL.md"]));
      }
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const toggleFile = (path: string) => {
    const next = new Set(expandedFiles);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setExpandedFiles(next);
  };

  const copyContent = async (path: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedFile(path);
      setTimeout(() => setCopiedFile(null), 2000);
    } catch {
      // Fallback
      const textarea = document.createElement("textarea");
      textarea.value = content;
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      document.body.removeChild(textarea);
      setCopiedFile(path);
      setTimeout(() => setCopiedFile(null), 2000);
    }
  };

  if (loading) {
    return (
      <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={onClose}>
        <div className="w-full max-w-3xl bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl p-8" onClick={e => e.stopPropagation()}>
          <div className="text-center text-slate-500">加载中...</div>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={onClose}>
        <div className="w-full max-w-3xl bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl p-8" onClick={e => e.stopPropagation()}>
          <div className="text-red-400 mb-4">加载失败: {error}</div>
          <button onClick={onClose} className="px-4 py-2 bg-white/5 rounded-lg text-white cursor-pointer">关闭</button>
        </div>
      </div>
    );
  }

  if (!skill) return null;

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={onClose}>
      <div className="w-full max-w-4xl bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-4 border-b border-white/5 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <FolderOpen className="w-5 h-5 text-violet-400" />
            <div>
              <h3 className="text-sm font-bold text-white">{skill.name}</h3>
              <p className="text-[10px] text-slate-500">{skill.files.length} 个文件</p>
            </div>
          </div>
          <button onClick={onClose} className="text-slate-500 hover:text-white cursor-pointer text-xs">关闭</button>
        </div>

        {/* File List */}
        <div className="flex-1 overflow-y-auto space-y-1 p-2">
          {skill.files.map(file => {
            const isExpanded = expandedFiles.has(file.path);
            const isMarkdown = file.path.endsWith(".md");
            const isCopied = copiedFile === file.path;

            return (
              <div key={file.path} className="rounded-lg border border-white/5 overflow-hidden">
                {/* File Header */}
                <div
                  className="flex items-center gap-2 px-3 py-2 bg-white/[0.02] hover:bg-white/[0.04] cursor-pointer transition-all"
                  onClick={() => toggleFile(file.path)}
                >
                  {isExpanded ? <ChevronDown className="w-3.5 h-3.5 text-slate-500" /> : <ChevronRight className="w-3.5 h-3.5 text-slate-500" />}
                  <FileText className="w-3.5 h-3.5 text-slate-400" />
                  <span className="text-xs text-slate-300 font-mono flex-1">{file.path}</span>
                  <button
                    onClick={(e) => { e.stopPropagation(); copyContent(file.path, file.contents); }}
                    className="p-1 rounded text-slate-600 hover:text-white hover:bg-white/10 cursor-pointer transition-all"
                    title="复制内容"
                  >
                    {isCopied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                  </button>
                </div>

                {/* File Content */}
                {isExpanded && (
                  <div className="border-t border-white/5">
                    {isMarkdown ? (
                      <div
                        className="p-4 text-sm text-slate-300 prose prose-invert max-w-none leading-relaxed [&_h1]:text-lg [&_h1]:font-bold [&_h1]:text-white [&_h1]:mb-2 [&_h2]:text-base [&_h2]:font-bold [&_h2]:text-white [&_h2]:mb-2 [&_h3]:text-sm [&_h3]:font-bold [&_h3]:text-white [&_h3]:mb-2 [&_p]:mb-2 [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:mb-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_ol]:mb-2 [&_li]:mb-1 [&_code]:bg-white/10 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-xs [&_pre]:bg-slate-900 [&_pre]:p-3 [&_pre]:rounded-lg [&_pre]:overflow-x-auto [&_pre]:mb-2 [&_pre>code]:bg-transparent [&_pre>code]:p-0 [&_blockquote]:border-l-2 [&_blockquote]:border-violet-500 [&_blockquote]:pl-3 [&_blockquote]:italic [&_a]:text-blue-400 [&_a]:underline"
                        dangerouslySetInnerHTML={{ __html: renderMarkdown(file.contents) }}
                      />
                    ) : (
                      <pre className="p-4 text-xs text-slate-300 font-mono overflow-x-auto whitespace-pre-wrap">
                        {file.contents}
                      </pre>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
