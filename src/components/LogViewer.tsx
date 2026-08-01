import { useEffect, useRef } from "react";
import { stripAnsi } from "../utils/ansi";

/**
 * 大文件日志查看器（纯前端 / 虚拟滚动）。
 * 逻辑移植自 pm2-log-viewer.html：
 * - 一次性读入文件字节（Uint8Array），构建行偏移索引 + 每行级别；
 * - 只渲染视口可见的 ~几十行（命令式虚拟滚动），支持 200MB+ 文件；
 * - 级别过滤 / 服务无关的搜索 / ANSI 着色 / Auto Tail。
 */
export default function LogViewer() {
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const $ = <T extends HTMLElement = HTMLElement>(sel: string) =>
      root.querySelector(sel) as T;

    // ---------- State ----------
    let bytes: Uint8Array | null = null;
    let offsets: Float64Array = new Float64Array(0);
    let levels: Uint8Array = new Uint8Array(0);
    let totalLines = 0;
    let maxLineBytes = 0;
    let errorCount = 0,
      warnCount = 0;
    let viewIndices: Uint32Array | null = null;
    let searchQuery = "";
    let searchActive = false;
    let searchList: number[] = [];
    let matchSet: Set<number> | null = null;
    let currentMatch = -1;
    let currentFile: File | null = null;
    let tailMode = false;
    let tailTimer: ReturnType<typeof setInterval> | null = null;
    let searchTimer: ReturnType<typeof setTimeout> | null = null;
    const decoder = new TextDecoder("utf-8");

    const RH = 21;
    const OVERSCAN = 12;
    let charWidth = 8;
    let flashRow = -1;

    const viewer = $("#lv-viewer") as HTMLDivElement;
    const sizer = $("#lv-sizer") as HTMLDivElement;
    let lastFirst = -1,
      lastLast = -1;

    // ---------- Helpers ----------
    const yieldFrame = () =>
      new Promise<void>((r) => requestAnimationFrame(() => r()));
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    function growTyped(arr: any, factor?: number) {
      const n = Math.max(1024, Math.floor(arr.length * (factor || 1.5)));
      const out = new arr.constructor(n);
      out.set(arr);
      return out;
    }
    const escapeHtml = (s: string) =>
      s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    const escapeRegex = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

    // ---------- Loading ----------
    async function loadFile(file?: File | null) {
      if (!file) return;
      currentFile = file;
      const overlay = $("#lv-overlay");
      overlay.style.display = "flex";
      try {
        const buf = await file.arrayBuffer();
        bytes = new Uint8Array(buf);
        await buildIndex();
        finishLoad(file.name, file.size, false);
      } catch (e: any) {
        $("#lv-overlayTxt").textContent = "读取失败: " + e.message;
        await sleep(1500);
      } finally {
        overlay.style.display = "none";
      }
    }

    async function buildIndex() {
      const b = bytes!;
      const n = b.length;
      const cap = Math.max(1024, Math.floor(n / 40));
      let off: any = new Float64Array(cap);
      let lv: any = new Uint8Array(cap);
      off[0] = 0;
      let count = 1;
      let lineStart = 0;
      let maxB = 0;
      let errs = 0,
        warns = 0;

      const setProgress = (p: number) => {
        ($("#lv-overlayBar") as HTMLElement).style.width =
          Math.floor(p * 100) + "%";
        $("#lv-overlayTxt").textContent =
          "正在解析文件… " + Math.floor(p * 100) + "%";
      };

      let i = 0;
      const CHUNK = 8 * 1024 * 1024;
      while (i < n) {
        const end = Math.min(i + CHUNK, n);
        for (; i < end; i++) {
          if (b[i] === 0x0a) {
            const len = i - lineStart;
            if (len > maxB) maxB = len;
            const lvl = classifyRange(b, lineStart, i);
            if (lvl === 2) errs++;
            else if (lvl === 1) warns++;
            lv[count - 1] = lvl;
            lineStart = i + 1;
            if (count >= off.length) off = growTyped(off);
            if (count >= lv.length) lv = growTyped(lv);
            off[count] = i + 1;
            count++;
          }
        }
        setProgress(i / n);
        await yieldFrame();
      }

      if (lineStart < n) {
        const len = n - lineStart;
        if (len > maxB) maxB = len;
        const lvl = classifyRange(b, lineStart, n);
        if (lvl === 2) errs++;
        else if (lvl === 1) warns++;
        lv[count - 1] = lvl;
      }

      if (count > 1 && off[count - 1] >= n) count--;

      offsets = off.subarray(0, count);
      levels = lv.subarray(0, count);
      totalLines = count;
      maxLineBytes = maxB;
      errorCount = errs;
      warnCount = warns;
    }

    function finishLoad(name: string, size: number, preserve: boolean) {
      $("#lv-dropzone").style.display = "none";
      viewer.style.display = "block";
      $("#lv-statusFile").textContent = name;
      $("#lv-statusSize").textContent = formatSize(size);

      measureCharWidth();
      root!.style.setProperty(
        "--lv-gutter-w",
        Math.max(48, String(totalLines).length * 8 + 14) + "px"
      );
      applyFilter(preserve);
      if (!preserve) {
        searchQuery = "";
        searchActive = false;
        searchList = [];
        matchSet = null;
        currentMatch = -1;
        ($("#lv-searchBox") as HTMLInputElement).value = "";
      }

      viewer.scrollTop = viewer.scrollHeight;
      render(true);
      updateStatus();
    }

    function measureCharWidth() {
      const cs = getComputedStyle(viewer);
      const span = document.createElement("span");
      span.style.cssText =
        "position:absolute;visibility:hidden;white-space:pre;font-family:" +
        cs.fontFamily +
        ";font-size:13px;font-weight:" +
        cs.fontWeight;
      span.textContent = "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM";
      document.body.appendChild(span);
      charWidth = span.getBoundingClientRect().width / 50 || 8;
      document.body.removeChild(span);
    }

    function formatSize(b: number) {
      if (b >= 1024 * 1024 * 1024)
        return (b / 1024 / 1024 / 1024).toFixed(2) + " GB";
      if (b >= 1024 * 1024) return (b / 1024 / 1024).toFixed(1) + " MB";
      if (b >= 1024) return (b / 1024).toFixed(1) + " KB";
      return b + " B";
    }

    // ---------- Level classification ----------
    const ERR_KW = [
      [101, 114, 114, 111, 114],
      [101, 120, 99, 101, 112, 116, 105, 111, 110],
      [102, 97, 105, 108],
      [102, 97, 116, 97, 108],
    ];
    const WARN_KW = [
      [119, 97, 114, 110],
      [119, 97, 114, 110, 105, 110, 103],
    ];
    function matchKW(b: Uint8Array, p: number, e: number, kw: number[]) {
      for (let k = 0; k < kw.length; k++) {
        const idx = p + k;
        if (idx >= e) return false;
        let c = b[idx];
        if (c >= 65 && c <= 90) c += 32;
        if (c !== kw[k]) return false;
      }
      return true;
    }
    function classifyRange(b: Uint8Array, s: number, e: number) {
      e = Math.min(e, s + 400);
      for (let p = s; p < e; p++) {
        let c = b[p];
        if (c >= 65 && c <= 90) c += 32;
        if (c === 101) {
          if (matchKW(b, p, e, ERR_KW[0]) || matchKW(b, p, e, ERR_KW[1]))
            return 2;
        } else if (c === 102) {
          if (matchKW(b, p, e, ERR_KW[2]) || matchKW(b, p, e, ERR_KW[3]))
            return 2;
        } else if (c === 119) {
          if (matchKW(b, p, e, WARN_KW[0]) || matchKW(b, p, e, WARN_KW[1]))
            return 1;
        }
      }
      return 0;
    }

    // ---------- Decode a single line ----------
    function decodeLine(lineIdx: number) {
      if (lineIdx < 0 || lineIdx >= totalLines) return "";
      const b = bytes!;
      const start = offsets[lineIdx];
      let end;
      if (lineIdx + 1 < totalLines) end = offsets[lineIdx + 1] - 1;
      else end = b.length;
      if (end > start && b[end - 1] === 0x0d) end--;
      return decoder.decode(b.subarray(start, end));
    }

    function cleanControl(text: string) {
      return text.replace(
        /[\x00-\x08\x0a-\x1a\x1c-\x1f\x7f]|\x1b(?![[[0-9;]*[A-Za-zm])/g,
        ""
      );
    }

    // ---------- ANSI parsing ----------
    const ANSI_RE = /\x1b\[([0-9;]*)([A-Za-z])/g;
    function handleSgr(seqs: string[], params: string) {
      if (params === "") {
        seqs.length = 0;
        return;
      }
      const codes = params.split(";").map(Number);
      for (const code of codes) {
        switch (code) {
          case 0:
            seqs.length = 0;
            break;
          case 1:
            seqs.push("ansi-bold");
            break;
          case 2:
            seqs.push("ansi-dim");
            break;
          case 3:
            seqs.push("ansi-italic");
            break;
          case 4:
            seqs.push("ansi-underline");
            break;
          case 22:
            removeSeq(seqs, "ansi-bold", "ansi-dim");
            break;
          case 23:
            removeSeq(seqs, "ansi-italic");
            break;
          case 24:
            removeSeq(seqs, "ansi-underline");
            break;
          case 39:
            removeColorSeq(seqs);
            break;
          default:
            if (code >= 30 && code <= 37) {
              removeColorSeq(seqs);
              seqs.push("ansi-" + (code - 30));
            } else if (code >= 90 && code <= 97) {
              removeColorSeq(seqs);
              seqs.push("ansi-" + (code - 90 + 8));
            }
        }
      }
    }
    function removeSeq(seqs: string[], ...names: string[]) {
      for (const nm of names) {
        const i = seqs.indexOf(nm);
        if (i >= 0) seqs.splice(i, 1);
      }
    }
    function removeColorSeq(seqs: string[]) {
      for (let i = seqs.length - 1; i >= 0; i--) {
        const s = seqs[i];
        if (
          s.startsWith("ansi-") &&
          !s.startsWith("ansi-bold") &&
          !s.startsWith("ansi-dim") &&
          !s.startsWith("ansi-italic") &&
          !s.startsWith("ansi-underline")
        ) {
          seqs.splice(i, 1);
        }
      }
    }
    function parseAnsi(text: string) {
      let result = "",
        last = 0;
      const seqs: string[] = [];
      ANSI_RE.lastIndex = 0;
      let m;
      while ((m = ANSI_RE.exec(text)) !== null) {
        if (m.index > last)
          result += wrap(seqs, escapeHtml(text.slice(last, m.index)));
        if (m[2] === "m") handleSgr(seqs, m[1]);
        last = ANSI_RE.lastIndex;
      }
      if (last < text.length) result += wrap(seqs, escapeHtml(text.slice(last)));
      return result;
    }
    function wrap(seqs: string[], esc: string) {
      const cls = seqs.join(" ");
      return cls ? `<span class="${cls}">${esc}</span>` : esc;
    }
    function highlightAll(html: string, q: string) {
      const re = new RegExp("(" + escapeRegex(q) + ")", "gi");
      return html.replace(/(<[^>]+>)|([^<]+)/g, (_m, tag, text) => {
        if (tag) return tag;
        return text.replace(re, '<span class="lv-highlight">$1</span>');
      });
    }

    // ---------- Filtering ----------
    function applyFilter(preserve?: boolean) {
      const level = ($("#lv-filterLevel") as HTMLSelectElement).value;
      if (level === "all") {
        viewIndices = null;
      } else {
        const want = level === "error" ? 2 : level === "warn" ? 1 : 0;
        let c = 0;
        for (let i = 0; i < totalLines; i++) {
          const l = levels[i];
          const ok = level === "info" ? l === 0 : l >= want;
          if (ok) c++;
        }
        const out = new Uint32Array(c);
        let k = 0;
        for (let i = 0; i < totalLines; i++) {
          const l = levels[i];
          const ok = level === "info" ? l === 0 : l >= want;
          if (ok) out[k++] = i;
        }
        viewIndices = out;
      }
      if (searchQuery && !preserve) runSearch(searchQuery);
      layout();
      if (!tailMode) viewer.scrollTop = 0;
      render(true);
      updateStatus();
    }

    const viewLen = () => (viewIndices ? viewIndices.length : totalLines);

    function layout() {
      const len = viewLen();
      sizer.style.height = len * RH + "px";
      const w = Math.ceil(maxLineBytes * charWidth * 1.6) + 80;
      sizer.style.width = w + "px";
    }

    // ---------- Virtual scrolling render ----------
    let renderScheduled = false;
    function scheduleRender() {
      if (renderScheduled) return;
      renderScheduled = true;
      requestAnimationFrame(() => {
        renderScheduled = false;
        render(false);
      });
    }
    function render(force: boolean) {
      if (!totalLines) return;
      const len = viewLen();
      const scrollTop = viewer.scrollTop;
      const vh = viewer.clientHeight;
      let first = Math.floor(scrollTop / RH) - OVERSCAN;
      let last = Math.ceil((scrollTop + vh) / RH) + OVERSCAN;
      first = Math.max(0, first);
      last = Math.min(len - 1, last);
      if (!force && first === lastFirst && last === lastLast) return;
      lastFirst = first;
      lastLast = last;

      sizer.innerHTML = "";
      const frag = document.createDocumentFragment();
      for (let row = first; row <= last; row++) {
        const lineIdx = viewIndices ? viewIndices[row] : row;
        let text = decodeLine(lineIdx);
        text = cleanControl(text);
        let html = parseAnsi(text);
        const div = document.createElement("div");
        div.className = "lv-line";
        div.dataset.row = String(row);
        div.dataset.line = String(lineIdx);
        const lvl = levels[lineIdx];
        if (lvl === 2) div.classList.add("error");
        else if (lvl === 1) div.classList.add("warn");

        if (searchActive && matchSet && matchSet.has(row)) {
          html = highlightAll(html, searchQuery);
          div.classList.add("search-match");
        }
        if (searchActive && searchList.length && row === searchList[currentMatch]) {
          div.classList.add("current-match");
        }
        if (row === flashRow) div.classList.add("lv-flashing");

        const gut = document.createElement("span");
        gut.className = "lv-gutter";
        gut.textContent = String(lineIdx + 1);

        const content = document.createElement("span");
        content.className = "lv-content";
        content.innerHTML = html;

        const copyBtn = document.createElement("button");
        copyBtn.className = "lv-copy";
        copyBtn.type = "button";
        copyBtn.title = "复制此行";
        copyBtn.textContent = "⧉";

        div.appendChild(gut);
        div.appendChild(content);
        div.appendChild(copyBtn);
        div.style.top = row * RH + "px";
        frag.appendChild(div);
      }
      sizer.appendChild(frag);
    }

    // ---------- Search ----------
    function onSearchInput() {
      const q = ($("#lv-searchBox") as HTMLInputElement).value;
      if (searchTimer) clearTimeout(searchTimer);
      if (!q.trim()) {
        clearSearch();
        return;
      }
      searchTimer = setTimeout(() => runSearch(q), 180);
    }
    async function runSearch(q: string) {
      searchQuery = q;
      const list: number[] = [];
      const len = viewLen();
      const re = new RegExp(escapeRegex(q), "i");
      let i = 0;
      const CHUNK = 20000;
      $("#lv-infoText").textContent = "搜索中…";
      while (i < len) {
        const end = Math.min(i + CHUNK, len);
        for (; i < end; i++) {
          const lineIdx = viewIndices ? viewIndices[i] : i;
          const text = cleanControl(decodeLine(lineIdx));
          if (re.test(text)) list.push(i);
        }
        await yieldFrame();
      }
      searchList = list;
      currentMatch = list.length ? 0 : -1;
      searchActive = true;
      matchSet = new Set(list);
      updateSearchInfo();
      render(true);
    }
    function clearSearch() {
      searchQuery = "";
      searchActive = false;
      searchList = [];
      matchSet = null;
      currentMatch = -1;
      $("#lv-infoText").textContent = "";
      render(true);
    }
    function searchNext() {
      if (!searchList.length) return;
      gotoMatch((currentMatch + 1) % searchList.length);
    }
    function searchPrev() {
      if (!searchList.length) return;
      gotoMatch((currentMatch - 1 + searchList.length) % searchList.length);
    }
    function gotoMatch(idx: number) {
      currentMatch = idx;
      const row = searchList[currentMatch];
      viewer.scrollTop = row * RH - viewer.clientHeight / 2;
      render(true);
      updateSearchInfo();
    }
    function updateSearchInfo() {
      const el = $("#lv-infoText");
      if (searchList.length)
        el.textContent = `${currentMatch + 1} / ${searchList.length}`;
      else if (searchQuery) el.textContent = "0 / 0";
      else el.textContent = "";
    }

    // ---------- Copy / Goto ----------
    function stripAnsi(s: string) {
      return s
        .replace(/\x1b\[[0-9;]*[A-Za-z]/g, "")
        .replace(/[\x00-\x08\x0a-\x0d\x0e-\x1f\x7f]/g, "");
    }
    function findRowForLine(lineIdx: number) {
      if (!viewIndices) return lineIdx;
      let lo = 0,
        hi = viewIndices.length - 1,
        res = -1;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        const v = viewIndices[mid];
        if (v === lineIdx) {
          res = mid;
          break;
        } else if (v < lineIdx) lo = mid + 1;
        else hi = mid - 1;
      }
      return res;
    }
    function flashRowAt(row: number) {
      flashRow = row;
      render(true);
      setTimeout(() => {
        if (flashRow === row) {
          flashRow = -1;
          render(true);
        }
      }, 700);
    }
    function gotoLine(n: number | string, depth = 0) {
      if (!totalLines) return;
      const num = parseInt(String(n).replace(/[^\d]/g, ""), 10);
      if (!Number.isFinite(num) || num < 1) return;
      const lineIdx = num - 1;
      if (lineIdx >= totalLines) return;
      let row = findRowForLine(lineIdx);
      if (row < 0) {
        if (depth === 0 && viewIndices) {
          ($("#lv-filterLevel") as HTMLSelectElement).value = "all";
          applyFilter();
          gotoLine(num, 1);
        }
        return;
      }
      viewer.scrollTop = row * RH - viewer.clientHeight / 2;
      flashRowAt(row);
    }
    function copyText(text: string, btn: HTMLElement) {
      const done = () => {
        const old = btn.textContent;
        btn.textContent = "✓";
        setTimeout(() => {
          btn.textContent = old;
        }, 1000);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done).catch(() => {
          fallbackCopy(text, done);
        });
      } else fallbackCopy(text, done);
    }
    function fallbackCopy(text: string, done: () => void) {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
        done();
      } catch {
        /* ignore */
      }
      document.body.removeChild(ta);
    }
    function onViewerClick(e: MouseEvent) {
      const btn = (e.target as HTMLElement).closest(".lv-copy") as HTMLElement | null;
      if (!btn) return;
      e.preventDefault();
      const lineEl = btn.closest(".lv-line") as HTMLElement | null;
      if (!lineEl) return;
      const lineIdx = parseInt(lineEl.dataset.line || "-1", 10);
      if (lineIdx < 0) return;
      copyText(stripAnsi(decodeLine(lineIdx)), btn);
    }

    // ---------- Status bar ----------
    function updateStatus() {
      const len = viewLen();
      $("#lv-statusLines").textContent = totalLines.toLocaleString() + " 行";
      $("#lv-statusErrors").textContent = errorCount.toLocaleString() + " errors";
      $("#lv-statusWarns").textContent = warnCount.toLocaleString() + " warnings";
      $("#lv-statusShown").textContent = viewIndices
        ? len.toLocaleString() + " shown"
        : "";
    }

    // ---------- Tail ----------
    function toggleTail() {
      tailMode = !tailMode;
      $("#lv-btnTail").classList.toggle("active", tailMode);
      if (tailMode && currentFile) {
        tailTimer = setInterval(tailTick, 3000);
        viewer.scrollTop = viewer.scrollHeight;
      } else if (tailTimer) {
        clearInterval(tailTimer);
        tailTimer = null;
      }
    }
    async function tailTick() {
      if (!currentFile || !tailMode) return;
      try {
        const buf = await currentFile.arrayBuffer();
        const newBytes = new Uint8Array(buf);
        if (bytes && newBytes.length === bytes.length) return;
        bytes = newBytes;
        await buildIndex();
        finishLoad(currentFile.name, currentFile.size, true);
        viewer.scrollTop = viewer.scrollHeight;
      } catch {
        /* ignore tail errors */
      }
    }

    // ---------- Events ----------
    const fileInput = $("#lv-fileInput") as HTMLInputElement;
    const dropzone = $("#lv-dropzone");
    const searchBox = $("#lv-searchBox") as HTMLInputElement;

    const onScroll = () => scheduleRender();
    const onResize = () => {
      lastFirst = lastLast = -1;
      layout();
      render(true);
    };
    const onFileChange = () => loadFile(fileInput.files?.[0]);
    const onDragOver = (e: DragEvent) => {
      e.preventDefault();
      dropzone.classList.add("dragover");
    };
    const onDragLeave = () => dropzone.classList.remove("dragover");
    const onDrop = (e: DragEvent) => {
      e.preventDefault();
      dropzone.classList.remove("dragover");
      const file = e.dataTransfer?.files?.[0];
      if (file) loadFile(file);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "f") {
        e.preventDefault();
        searchBox.focus();
      }
    };
    const onSearchKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        if (e.shiftKey) searchPrev();
        else searchNext();
      }
    };

    // 工具栏按钮
    const onOpenClick = () => fileInput.click();
    const onGotoClick = () =>
      gotoLine(($("#lv-gotoInput") as HTMLInputElement).value);
    const onGotoKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        gotoLine(($("#lv-gotoInput") as HTMLInputElement).value);
      }
    };
    $("#lv-btnOpen").addEventListener("click", onOpenClick);
    $("#lv-btnGoto").addEventListener("click", onGotoClick);
    $("#lv-gotoInput").addEventListener("keydown", onGotoKey);
    $("#lv-btnTail").addEventListener("click", toggleTail);
    $("#lv-btnPrev").addEventListener("click", searchPrev);
    $("#lv-btnNext").addEventListener("click", searchNext);
    ($("#lv-filterLevel") as HTMLSelectElement).addEventListener(
      "change",
      () => applyFilter()
    );
    searchBox.addEventListener("input", onSearchInput);
    searchBox.addEventListener("keydown", onSearchKey);
    fileInput.addEventListener("change", onFileChange);
    viewer.addEventListener("scroll", onScroll);
    viewer.addEventListener("click", onViewerClick);
    root.addEventListener("dragover", onDragOver);
    root.addEventListener("dragleave", onDragLeave);
    root.addEventListener("drop", onDrop);
    root.addEventListener("keydown", onKeyDown);

    const ro = new ResizeObserver(onResize);
    ro.observe(viewer);
    window.addEventListener("resize", onResize);

    return () => {
      if (tailTimer) clearInterval(tailTimer);
      if (searchTimer) clearTimeout(searchTimer);
      $("#lv-btnOpen")?.removeEventListener("click", onOpenClick);
      $("#lv-btnGoto")?.removeEventListener("click", onGotoClick);
      $("#lv-gotoInput")?.removeEventListener("keydown", onGotoKey);
      searchBox?.removeEventListener("input", onSearchInput);
      searchBox?.removeEventListener("keydown", onSearchKey);
      fileInput?.removeEventListener("change", onFileChange);
      viewer?.removeEventListener("scroll", onScroll);
      viewer?.removeEventListener("click", onViewerClick);
      root.removeEventListener("dragover", onDragOver);
      root.removeEventListener("dragleave", onDragLeave);
      root.removeEventListener("drop", onDrop);
      root.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("resize", onResize);
      ro.disconnect();
    };
  }, []);

  return (
    <div className="lv-root" ref={rootRef} tabIndex={-1}>
      <style>{CSS}</style>

      <div className="lv-toolbar">
        <span className="lv-title">日志查看器</span>
        <button className="lv-btn" id="lv-btnOpen">
          打开文件
        </button>
        <input
          type="file"
          id="lv-fileInput"
          accept=".log,.txt,.out,*"
          style={{ display: "none" }}
        />
        <button className="lv-btn" id="lv-btnTail">
          Auto Tail
        </button>
        <select className="lv-filter-select" id="lv-filterLevel" defaultValue="all">
          <option value="all">全部</option>
          <option value="error">仅错误</option>
          <option value="warn">错误 + 警告</option>
          <option value="info">仅普通</option>
        </select>
        <input
          className="lv-search-box"
          id="lv-searchBox"
          placeholder="搜索... (Ctrl+F)"
        />
        <button className="lv-btn" id="lv-btnPrev" title="上一个">
          &#9650;
        </button>
        <button className="lv-btn" id="lv-btnNext" title="下一个">
          &#9660;
        </button>
        <span className="lv-goto">
          跳至
          <input
            className="lv-goto-input"
            id="lv-gotoInput"
            type="number"
            min="1"
            placeholder="行号"
          />
          <button className="lv-btn" id="lv-btnGoto">
            行
          </button>
        </span>
        <span className="lv-info" id="lv-infoText"></span>
      </div>

      <div className="lv-dropzone" id="lv-dropzone">
        <div style={{ textAlign: "center" }}>
          <div className="lv-icon">&#128196;</div>
          <div>拖拽日志文件到此处</div>
          <div style={{ fontSize: 12, marginTop: 6 }}>
            或点击「打开文件」选择 &nbsp;·&nbsp; 支持 200MB+ 大文件
          </div>
        </div>
      </div>

      <div id="lv-viewer">
        <div id="lv-sizer"></div>
      </div>

      <div className="lv-statusbar">
        <span id="lv-statusFile">未打开文件</span>
        <span id="lv-statusLines">0 行</span>
        <span id="lv-statusErrors" className="lv-badge lv-badge-error">
          0 errors
        </span>
        <span id="lv-statusWarns" className="lv-badge lv-badge-warn">
          0 warnings
        </span>
        <span id="lv-statusSize"></span>
        <span id="lv-statusShown"></span>
      </div>

      <div id="lv-overlay">
        <div className="lv-txt" id="lv-overlayTxt">
          正在解析文件…
        </div>
        <div className="lv-bar">
          <i id="lv-overlayBar"></i>
        </div>
      </div>
    </div>
  );
}

const CSS = `
.lv-root {
  --lv-bg: #0d111d; --lv-surface: #0e1220; --lv-border: rgba(255,255,255,0.06);
  --lv-text: #e2e8f0; --lv-muted: #94a3b8; --lv-accent: #10b981;
  --lv-error: #f87171; --lv-warn: #fbbf24; --lv-scrollbar: #1e293b;
  --lv-scrollbar-thumb: #334155; --lv-rowh: 21px;
  --lv-mono: 'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', 'Courier New', monospace;
  position: relative; height: 100%; display: flex; flex-direction: column;
  overflow: hidden; background: var(--lv-bg); color: var(--lv-text);
  font-family: var(--lv-mono); font-size: 13px; outline: none;
  color-scheme: dark;
}
.lv-root * { box-sizing: border-box; }

.lv-toolbar {
  background: var(--lv-surface); border-bottom: 1px solid var(--lv-border);
  padding: 8px 16px; display: flex; align-items: center; gap: 10px;
  flex-wrap: wrap; flex-shrink: 0;
}
.lv-title { font-weight: 700; font-size: 14px; color: var(--lv-accent); margin-right: 8px; }

.lv-btn {
  background: rgba(255,255,255,0.06); color: var(--lv-text);
  border: 1px solid rgba(255,255,255,0.08); padding: 5px 12px; border-radius: 4px;
  cursor: pointer; font-family: inherit; font-size: 12px; transition: background 0.15s;
}
.lv-btn:hover { background: rgba(255,255,255,0.12); }
.lv-btn.active { background: var(--lv-accent); color: #06281d; border-color: var(--lv-accent); }

.lv-search-box {
  background: var(--lv-bg); border: 1px solid var(--lv-border); color: var(--lv-text);
  padding: 5px 10px; border-radius: 4px; font-family: inherit; font-size: 12px;
  width: 220px; outline: none;
}
.lv-search-box:focus { border-color: var(--lv-accent); }
.lv-search-box::placeholder { color: var(--lv-muted); }

.lv-info { color: var(--lv-muted); font-size: 11px; margin-left: auto; white-space: nowrap; }

.lv-goto { display: flex; align-items: center; gap: 4px; color: var(--lv-muted); font-size: 12px; }
.lv-goto-input {
  width: 72px; background: var(--lv-bg); border: 1px solid var(--lv-border);
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
  flex: 1; overflow: auto; position: relative; display: none; background: var(--lv-bg);
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
  background: var(--lv-surface); border-top: 1px solid var(--lv-border);
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
.lv-filter-select option { background: #0e1220; color: var(--lv-text); }

.lv-root #lv-overlay {
  position: absolute; inset: 0; background: rgba(13,17,29,0.85);
  display: none; align-items: center; justify-content: center;
  flex-direction: column; gap: 14px; z-index: 50; color: var(--lv-text);
}
.lv-root #lv-overlay .lv-bar { width: 280px; height: 6px; background: var(--lv-border); border-radius: 3px; overflow: hidden; }
.lv-root #lv-overlay .lv-bar > i { display: block; height: 100%; width: 0; background: var(--lv-accent); transition: width 0.1s; }
.lv-root #lv-overlay .lv-txt { font-size: 13px; color: var(--lv-muted); }

/* 自定义滚动条 */
.lv-root #lv-viewer::-webkit-scrollbar { width: 12px; height: 12px; }
.lv-root #lv-viewer::-webkit-scrollbar-track { background: var(--lv-bg); }
.lv-root #lv-viewer::-webkit-scrollbar-thumb { background: var(--lv-scrollbar-thumb); border-radius: 6px; }
`;
