export const CSS = `
.lv-root {
  --lv-bg: var(--color-surface); --lv-surface: var(--color-surface-topbar); --lv-border: rgba(255,255,255,0.06);
  --lv-text: #e2e8f0; --lv-muted: #94a3b8; --lv-accent: var(--module-accent);
  --lv-error: #f87171; --lv-warn: #fbbf24; --lv-scrollbar: #1e293b;
  --lv-scrollbar-thumb: #334155; --lv-rowh: 21px;
  --lv-mono: 'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', 'Courier New', monospace;
  position: relative; height: 100%; display: flex; flex-direction: column;
  overflow: hidden; background: transparent; color: var(--lv-text);
  font-family: var(--lv-mono); font-size: 13px; outline: none;
  color-scheme: dark;
}
.lv-root * { box-sizing: border-box; }

.lv-toolbar {
  background: rgba(255,255,255,0.03); border-bottom: 1px solid var(--lv-border);
  padding: 8px 16px; display: flex; align-items: center; gap: 10px;
  flex-wrap: wrap; flex-shrink: 0;
}
.lv-title { font-weight: 700; font-size: 13px; color: var(--lv-accent); margin-right: 8px; }

.lv-btn {
  background: rgba(255,255,255,0.06); color: var(--lv-text);
  border: 1px solid rgba(255,255,255,0.08); padding: 5px 12px; border-radius: 4px;
  cursor: pointer; font-family: inherit; font-size: 12px; transition: background 0.15s;
}
.lv-btn:hover { background: rgba(255,255,255,0.12); }
.lv-root #lv-btnTail.active { background: var(--lv-accent); color: #fff; border-color: var(--lv-accent); }

.lv-search-box {
  background: rgba(0,0,0,0.3); border: 1px solid var(--lv-border); color: var(--lv-text);
  padding: 5px 10px; border-radius: 4px; font-family: inherit; font-size: 12px;
  width: 220px; outline: none;
}
.lv-search-box:focus { border-color: var(--lv-accent); }
.lv-search-box::placeholder { color: var(--lv-muted); }

.lv-info { color: var(--lv-muted); font-size: 11px; margin-left: auto; white-space: nowrap; }

.lv-goto { display: flex; align-items: center; gap: 4px; color: var(--lv-muted); font-size: 12px; }
.lv-goto-input {
  width: 72px; background: rgba(0,0,0,0.3); border: 1px solid var(--lv-border);
  color: var(--lv-text); padding: 5px 8px; border-radius: 4px; font-family: inherit;
  font-size: 12px; outline: none;
}
.lv-goto-input:focus { border-color: var(--lv-accent); }
.lv-goto-input::-webkit-outer-spin-button,
.lv-goto-input::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }

.lv-dropzone {
  flex: 1; display: flex; align-items: center; justify-content: center;
  border: 2px dashed var(--lv-border); margin: 20px; border-radius: 8px;
  color: var(--lv-muted); font-size: 16px; transition: all 0.2s;
}
.lv-dropzone.dragover { border-color: var(--lv-accent); background: rgba(137,180,250,0.05); color: var(--lv-accent); }
.lv-dropzone .lv-icon { font-size: 48px; margin-bottom: 12px; }

.lv-root #lv-viewer {
  flex: 1; overflow: auto; position: relative; display: none; background: transparent;
}
.lv-root #lv-sizer { position: relative; }

.lv-line {
  position: absolute; left: 0; white-space: pre; padding: 0;
  height: var(--lv-rowh); line-height: var(--lv-rowh); font-size: 13px;
  border-left: 3px solid transparent; will-change: transform; contain: layout style;
  display: flex; align-items: stretch; min-width: max-content;
}
.lv-gutter {
  flex: 0 0 auto; width: var(--lv-gutter-w, 58px); text-align: right;
  padding-right: 12px; color: var(--lv-muted); user-select: none;
  border-right: 1px solid var(--lv-border); background: rgba(0,0,0,0.18);
}
.lv-content {
  flex: 0 1 auto; white-space: pre; padding: 0 12px; user-select: text;
}
.lv-copy {
  position: absolute; right: 8px; top: 0; height: var(--lv-rowh); width: 26px;
  border: none; background: transparent; color: var(--lv-accent); cursor: pointer;
  opacity: 0; font-size: 14px; line-height: var(--lv-rowh); z-index: 2; padding: 0;
}
.lv-line:hover .lv-copy { opacity: 1; }
.lv-copy:hover { color: #fff; background: rgba(137,180,250,0.22); border-radius: 4px; }
.lv-line:hover { background: rgba(255,255,255,0.04); }
.lv-line.error { border-left-color: var(--lv-error); background: rgba(248,113,113,0.07); }
.lv-line.warn  { border-left-color: var(--lv-warn);  background: rgba(251,191,36,0.07); }
.lv-line.search-match { background: rgba(16,185,129,0.12); }
.lv-line.current-match { background: rgba(16,185,129,0.30) !important; border-left-color: var(--lv-accent); }
.lv-line.lv-flashing { background: rgba(251,191,36,0.30) !important; border-left-color: var(--lv-warn) !important; }
.lv-line .lv-highlight { background: rgba(16,185,129,0.45); color: #06281d; border-radius: 2px; padding: 0 1px; }

.lv-root .ansi-0{color:#6c7086}.lv-root .ansi-1{color:#f38ba8;font-weight:bold}.lv-root .ansi-2{color:#a6e3a1}
.lv-root .ansi-3{color:#f9e2af}.lv-root .ansi-4{color:#89b4fa}.lv-root .ansi-5{color:#f5c2e7}.lv-root .ansi-6{color:#94e2d5}
.lv-root .ansi-7{color:#cdd6f4}.lv-root .ansi-8{color:#45475a}.lv-root .ansi-9{color:#f38ba8}.lv-root .ansi-10{color:#a6e3a1}
.lv-root .ansi-11{color:#f9e2af}.lv-root .ansi-12{color:#89b4fa}.lv-root .ansi-13{color:#f5c2e7}.lv-root .ansi-14{color:#94e2d5}.lv-root .ansi-15{color:#cdd6f4}
.lv-root .ansi-underline{text-decoration:underline}.lv-root .ansi-bold{font-weight:bold}.lv-root .ansi-dim{opacity:.6}.lv-root .ansi-italic{font-style:italic}

.lv-statusbar {
  background: rgba(255,255,255,0.03); border-top: 1px solid var(--lv-border);
  padding: 4px 16px; display: flex; align-items: center; gap: 16px;
  font-size: 11px; color: var(--lv-muted); flex-shrink: 0;
}
.lv-badge { padding: 1px 8px; border-radius: 3px; font-size: 10px; }
.lv-badge-error { background: rgba(248,113,113,0.18); color: var(--lv-error); }
.lv-badge-warn  { background: rgba(251,191,36,0.18); color: var(--lv-warn); }

.lv-filter-select {
  background: rgba(255,255,255,0.06); border: 1px solid rgba(255,255,255,0.08); color: var(--lv-text);
  padding: 5px 8px; border-radius: 4px; font-family: inherit; font-size: 12px; outline: none;
}
.lv-filter-select:focus { border-color: var(--lv-accent); }
.lv-filter-select option { background: var(--color-surface-topbar); color: var(--lv-text); }

.lv-root #lv-overlay {
  position: absolute; inset: 0; background: color-mix(in srgb, var(--color-surface) 85%, transparent);
  display: none; align-items: center; justify-content: center;
  flex-direction: column; gap: 14px; z-index: 50; color: var(--lv-text);
}
.lv-root #lv-overlay .lv-bar { width: 280px; height: 6px; background: var(--lv-border); border-radius: 3px; overflow: hidden; }
.lv-root #lv-overlay .lv-bar > i { display: block; height: 100%; width: 0; background: var(--lv-accent); transition: width 0.1s; }
.lv-root #lv-overlay .lv-txt { font-size: 13px; color: var(--lv-muted); }

/* 自定义滚动条 */
.lv-root #lv-viewer::-webkit-scrollbar { width: 12px; height: 12px; }
.lv-root #lv-viewer::-webkit-scrollbar-track { background: transparent; }
.lv-root #lv-viewer::-webkit-scrollbar-thumb { background: var(--lv-scrollbar-thumb); border-radius: 6px; }
`;
