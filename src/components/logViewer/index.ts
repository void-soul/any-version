export const DEFAULT_LOG_LIMITS = { maxBytes: 256 * 1024 * 1024, maxLines: 1_000_000, maxLineBytes: 1024 * 1024 };
function growTyped(arr: any, max: number) {
  const n = Math.min(max, Math.max(1024, Math.floor(arr.length * 1.5)));
  const out = new arr.constructor(n);
  out.set(arr);
  return out;
}

export async function buildLogIndex(b: Uint8Array, yieldFrame: () => Promise<void>, isCurrent: () => boolean, onProgress: (progress: number) => void, limits = DEFAULT_LOG_LIMITS) {
  const n = b.length;
  if (n > limits.maxBytes) throw new Error("logTooLarge");
  if (!isCurrent()) return null;
  const cap = Math.min(limits.maxLines + 1, Math.max(1024, Math.floor(n / 40)));
  let off: any = new Uint32Array(cap);
  let lv: any = new Uint8Array(cap);
  off[0] = 0;
  let count = 1;
  let lineStart = 0;
  let maxB = 0;
  let errs = 0,
    warns = 0;

  let i = 0;
  const CHUNK = 8 * 1024 * 1024;
  while (i < n) {
    const end = Math.min(i + CHUNK, n);
    for (; i < end; i++) {
      if (i - lineStart > limits.maxLineBytes) throw new Error("logLineTooLong");
      if (b[i] === 0x0a) {
        const len = i - lineStart;
        if (len > maxB) maxB = len;
        const lvl = classifyRange(b, lineStart, i);
        if (lvl === 2) errs++;
        else if (lvl === 1) warns++;
        lv[count - 1] = lvl;
        lineStart = i + 1;
        if (count >= limits.maxLines && lineStart < n) throw new Error("logTooManyLines");
        if (count >= off.length) off = growTyped(off, limits.maxLines + 1);
        if (count >= lv.length) lv = growTyped(lv, limits.maxLines + 1);
        off[count] = i + 1;
        count++;
      }
    }
    onProgress(i / n);
    await yieldFrame();
    if (!isCurrent()) return null;
  }

  if (n - lineStart > limits.maxLineBytes) throw new Error("logLineTooLong");
  if (lineStart < n) {
    const len = n - lineStart;
    if (len > maxB) maxB = len;
    const lvl = classifyRange(b, lineStart, n);
    if (lvl === 2) errs++;
    else if (lvl === 1) warns++;
    lv[count - 1] = lvl;
  }

  if (count > 1 && off[count - 1] >= n) count--;

  if (n === 0) count = 0;
  return { offsets: off.slice(0, count) as Uint32Array, levels: lv.slice(0, count) as Uint8Array, totalLines: count, maxLineBytes: maxB, errorCount: errs, warnCount: warns };
}

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
