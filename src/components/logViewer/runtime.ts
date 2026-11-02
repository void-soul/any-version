import { buildLogIndex, DEFAULT_LOG_LIMITS } from "./index";
import type { TFunction } from "i18next";

export function mountLogViewer(root: HTMLDivElement, t: TFunction) {
  const $ = <T extends HTMLElement = HTMLElement>(sel: string) =>
    root.querySelector(sel) as T;

  // ---------- State ----------
  let bytes: Uint8Array | null = null;
  let offsets: Uint32Array = new Uint32Array(0);
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
  let disposed = false;
  let loadRequest = 0;
  let searchRequest = 0;
  let loadingFile = false;
  let queuedFile: File | null = null;
  let tailPending = false;
  const feedbackTimers = new Set<ReturnType<typeof setTimeout>>();
  const frameWaiters = new Map<number, () => void>();
  let renderFrame: number | null = null;
  const after = (callback: () => void, ms: number) => {
    const timer = setTimeout(() => { feedbackTimers.delete(timer); if (!disposed) callback(); }, ms);
    feedbackTimers.add(timer);
  };
  const showError = (error: unknown) => {
    if (disposed) return;
    const code = error instanceof Error ? error.message : String(error);
    const key = ({ logTooLarge: "logview.fileTooLarge", logTooManyLines: "logview.tooManyLines", logLineTooLong: "logview.lineTooLong" } as Record<string, string>)[code];
    const message = $("#lv-error");
    message.textContent = key ? t(key) : t("logview.readFail", { err: code });
    message.style.display = "block";
  };

  const RH = 21;
  const OVERSCAN = 12;
  let charWidth = 8;
  let flashRow = -1;

  const viewer = $("#lv-viewer") as HTMLDivElement;
  const sizer = $("#lv-sizer") as HTMLDivElement;
  let lastFirst = -1,
    lastLast = -1;

  // ---------- Helpers ----------
  const yieldFrame = () => new Promise<void>((resolve) => {
    if (disposed) { resolve(); return; }
    const frame = requestAnimationFrame(() => { frameWaiters.delete(frame); resolve(); });
    frameWaiters.set(frame, resolve);
  });
  const escapeHtml = (s: string) =>
    s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const escapeRegex = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

  // ---------- Loading ----------
  async function indexFile(file: File, request: number) {
    if (file.size > DEFAULT_LOG_LIMITS.maxBytes) throw new Error("logTooLarge");
    const buf = await file.arrayBuffer();
    if (disposed || request !== loadRequest) return false;
    const nextBytes = new Uint8Array(buf);
    const index = await buildLogIndex(nextBytes, yieldFrame, () => !disposed && request === loadRequest, (progress) => {
      if (disposed || request !== loadRequest) return;
      $("#lv-overlayBar").style.width = Math.floor(progress * 100) + "%";
      $("#lv-overlayTxt").textContent = t("logview.parsingFile", { pct: Math.floor(progress * 100) });
    });
    if (!index || disposed || request !== loadRequest) return false;
    bytes = nextBytes; offsets = index.offsets; levels = index.levels;
    totalLines = index.totalLines; maxLineBytes = index.maxLineBytes;
    errorCount = index.errorCount; warnCount = index.warnCount;
    return true;
  }
  async function loadFile(file?: File | null) {
    if (!file || disposed) return;
    queuedFile = file;
    searchRequest++;
    if (loadingFile) { loadRequest++; return; }
    loadingFile = true;
    const overlay = $("#lv-overlay");
    overlay.style.display = "flex";
    try {
      while (queuedFile && !disposed) {
        const next: File = queuedFile;
        queuedFile = null;
        const request = ++loadRequest;
        $("#lv-error").style.display = "none";
        try {
          if (!await indexFile(next, request)) continue;
          currentFile = next;
          finishLoad(next.name, next.size, false);
        } catch (error) { if (request === loadRequest) showError(error); }
      }
    } finally {
      loadingFile = false;
      if (!disposed) overlay.style.display = "none";
    }
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
    searchRequest++;
    if (!preserve) {
      searchQuery = "";
      searchActive = false;
      searchList = [];
      matchSet = null;
      currentMatch = -1;
      ($("#lv-searchBox") as HTMLInputElement).value = "";
    }

    applyFilter(preserve);
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
  function applyFilter(_preserve?: boolean) {
    if (disposed) return;
    searchRequest++;
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
    if (searchQuery) void runSearch(searchQuery);
    else { searchActive = false; searchList = []; matchSet = null; currentMatch = -1; }
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
    renderFrame = requestAnimationFrame(() => {
      renderFrame = null;
      renderScheduled = false;
      if (!disposed) render(false);
    });
  }
  function render(force: boolean) {
    if (disposed || !totalLines) { if (!disposed) sizer.innerHTML = ""; return; }
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
      copyBtn.title = t("logview.copyLine");
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
    searchRequest++;
    const q = ($("#lv-searchBox") as HTMLInputElement).value;
    if (searchTimer) clearTimeout(searchTimer);
    if (!q.trim()) {
      clearSearch();
      return;
    }
    searchTimer = setTimeout(() => runSearch(q), 180);
  }
  async function runSearch(q: string) {
    if (disposed || loadingFile) return;
    const request = ++searchRequest;
    searchQuery = q;
    const list: number[] = [];
    const len = viewLen();
    const re = new RegExp(escapeRegex(q), "i");
    let i = 0;
    const CHUNK = 20000;
    $("#lv-infoText").textContent = t("logview.searching");
    while (i < len) {
      const end = Math.min(i + CHUNK, len);
      for (; i < end; i++) {
        const lineIdx = viewIndices ? viewIndices[i] : i;
        const text = cleanControl(decodeLine(lineIdx));
        if (re.test(text)) list.push(i);
      }
      await yieldFrame();
      if (disposed || request !== searchRequest) return;
    }
    searchList = list;
    currentMatch = list.length ? 0 : -1;
    searchActive = true;
    matchSet = new Set(list);
    updateSearchInfo();
    render(true);
  }
  function clearSearch() {
    searchRequest++;
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
    after(() => {
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
      if (disposed) return;
      const old = btn.textContent;
      btn.textContent = "✓";
      after(() => {
        btn.textContent = old;
      }, 1000);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done).catch(() => {
        if (!disposed) fallbackCopy(text, done);
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
      if (!document.execCommand("copy")) throw new Error(t("logview.copyFail"));
      done();
    } catch {
      showError(new Error(t("logview.copyFail")));
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
    $("#lv-statusLines").textContent = t("logview.lines", { n: totalLines.toLocaleString() });
    $("#lv-statusErrors").textContent = t("logview.errorCount", { n: errorCount.toLocaleString() });
    $("#lv-statusWarns").textContent = t("logview.warnCount", { n: warnCount.toLocaleString() });
    $("#lv-statusShown").textContent = viewIndices
      ? t("logview.shownCount", { n: len.toLocaleString() })
      : "";
  }

  // ---------- Tail ----------
  function toggleTail() {
    tailMode = !tailMode;
    $("#lv-btnTail").classList.toggle("active", tailMode);
    $("#lv-btnTail").setAttribute("aria-pressed", String(tailMode));
    if (tailMode) {
      tailTimer = setInterval(tailTick, 3000);
      viewer.scrollTop = viewer.scrollHeight;
    } else if (tailTimer) {
      clearInterval(tailTimer);
      tailTimer = null;
    }
  }
  async function tailTick() {
    if (disposed || document.hidden || !currentFile || !tailMode || loadingFile || tailPending) return;
    tailPending = true;
    const request = loadRequest;
    const file = currentFile;
    try {
      const buf = await file.slice(0, DEFAULT_LOG_LIMITS.maxBytes + 1).arrayBuffer();
      if (disposed || request !== loadRequest || !tailMode) return;
      if (bytes && buf.byteLength === bytes.length) return;
      searchRequest++;
      // Reuse the already read bytes rather than reading the same file twice.
      const snapshot = { size: buf.byteLength, arrayBuffer: async () => buf } as File;
      if (!await indexFile(snapshot, request)) return;
      finishLoad(file.name, buf.byteLength, true);
      viewer.scrollTop = viewer.scrollHeight;
    } catch (error) { if (request === loadRequest) showError(error); }
    finally { tailPending = false; }
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
  const onFileChange = () => { void loadFile(fileInput.files?.[0]); fileInput.value = ""; };
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
  const onFilterChange = () => applyFilter();
  $("#lv-filterLevel").addEventListener("change", onFilterChange);
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
    disposed = true; loadRequest++; searchRequest++; tailMode = false;
    feedbackTimers.forEach((timer) => clearTimeout(timer)); feedbackTimers.clear();
    frameWaiters.forEach((resolve, frame) => { cancelAnimationFrame(frame); resolve(); }); frameWaiters.clear();
    if (renderFrame !== null) cancelAnimationFrame(renderFrame);
    $("#lv-btnTail").removeEventListener("click", toggleTail);
    $("#lv-btnPrev").removeEventListener("click", searchPrev);
    $("#lv-btnNext").removeEventListener("click", searchNext);
    $("#lv-filterLevel").removeEventListener("change", onFilterChange);
    bytes = null; offsets = new Uint32Array(0); levels = new Uint8Array(0); viewIndices = null;
    searchList = []; matchSet = null; currentFile = null; queuedFile = null; sizer.replaceChildren();
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
}
