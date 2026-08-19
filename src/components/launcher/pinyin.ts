/**
 * 拼音首字母搜索工具（基于 pinyin-pro）
 *
 * 用于快速搜索时支持「首字母」匹配：
 * - 中文名称：输入 微信 的 wx 命中；输入全拼 weixin / wei xin 也能命中；
 * - 英文/混合名称：输入 Visual Studio Code 的 vsc 命中（按单词首字母）。
 */
import { pinyin } from "pinyin-pro";

/** 结果缓存，避免同一名称在每次搜索时重复计算拼音。 */
const initialsCache = new Map<string, string>();
const fullCache = new Map<string, string>();

/** 是否为 CJK 汉字。 */
function isHanzi(ch: string): boolean {
  const code = ch.charCodeAt(0);
  return code >= 0x4e00 && code <= 0x9fff;
}

/** 是否为 ASCII 字母。 */
function isAsciiLetter(ch: string): boolean {
  const c = ch.charCodeAt(0);
  return (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a);
}

/** 是否为 ASCII 数字。 */
function isAsciiDigit(ch: string): boolean {
  const c = ch.charCodeAt(0);
  return c >= 0x30 && c <= 0x39;
}

/**
 * 将文本转为「首字母」串：
 * - 汉字取拼音首字母（微信 → WX）；
 * - 英文取每个单词的首字母（Visual Studio Code → VSC，VSCode → V）；
 * - 数字保留（QQ 2010 → QQ2010）。
 */
export function pinyinInitials(text: string): string {
  const cached = initialsCache.get(text);
  if (cached !== undefined) return cached;

  let out = "";
  let wordStart = true; // 是否处于「新单词」起始（紧邻空白/标点）
  for (const ch of text) {
    if (isHanzi(ch)) {
      out += pinyin(ch, { pattern: "first", toneType: "none", type: "array" })[0].toUpperCase();
      wordStart = false;
    } else if (isAsciiLetter(ch)) {
      if (wordStart) out += ch.toUpperCase();
      wordStart = false;
    } else if (isAsciiDigit(ch)) {
      if (wordStart) out += ch;
      wordStart = false;
    } else {
      // 空格 / 标点等：作为单词边界
      wordStart = true;
    }
  }
  initialsCache.set(text, out);
  return out;
}

/** 将文本转为小写全拼串（去空白），支持中文全拼匹配。 */
export function pinyinFull(text: string): string {
  const cached = fullCache.get(text);
  if (cached !== undefined) return cached;
  const out = pinyin(text, {
    pattern: "pinyin",
    toneType: "none",
    type: "array",
  })
    .join("")
    .toLowerCase();
  fullCache.set(text, out);
  return out;
}

/**
 * 判断 query（已转小写）是否命中文本：
 * 1. 子串匹配（名称/目标）；2. 首字母匹配（中文拼音首字母 / 英文单词首字母）；3. 中文全拼匹配。
 */
export function matchPinyin(text: string, query: string): boolean {
  const q = query.toLowerCase();
  if (text.toLowerCase().includes(q)) return true;
  if (pinyinInitials(text).toLowerCase().includes(q)) return true;
  if (pinyinFull(text).includes(q)) return true;
  return false;
}
