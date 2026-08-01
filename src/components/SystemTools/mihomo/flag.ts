/**
 * 节点名 → 国家/地区代码（flag-icons 的 class 后缀，小写 ISO 3166-1 alpha-2）
 * 规则：优先识别节点名里已带的 emoji 国旗，其次按中英文关键字匹配。
 */

/** emoji 国旗（U+1F1E6..U+1F1FF 两个字符）转 ISO2 */
function fromEmoji(name: string): string | null {
  for (const ch of name) {
    const cp = ch.codePointAt(0) ?? 0;
    if (cp >= 0x1f1e6 && cp <= 0x1f1ff) {
      const idx = name.indexOf(ch);
      const next = name.codePointAt(idx + ch.length) ?? 0;
      if (next >= 0x1f1e6 && next <= 0x1f1ff) {
        const a = String.fromCharCode(cp - 0x1f1e6 + 65);
        const b = String.fromCharCode(next - 0x1f1e6 + 65);
        return (a + b).toLowerCase();
      }
    }
  }
  return null;
}

/** 关键字 → ISO2，靠前的优先（顺序敏感：先长后短，避免 "中国" 命中 "中转"） */
const RULES: [RegExp, string][] = [
  [/香港|hong ?kong|\bhk\b|hkg/i, "hk"],
  [/澳门|澳門|macao|macau|\bmo\b/i, "mo"],
  [/台湾|台灣|taiwan|\btw\b|tpe/i, "tw"],
  [/日本|东京|大阪|japan|\bjp\b|tokyo|osaka/i, "jp"],
  [/新加坡|狮城|singapore|\bsg\b/i, "sg"],
  [/韩国|韓國|首尔|korea|\bkr\b|seoul/i, "kr"],
  [/美国|美國|硅谷|洛杉矶|圣何塞|西雅图|芝加哥|纽约|united ?states|\bus\b|usa|los ?angeles|san ?jose|seattle|new ?york/i, "us"],
  [/英国|英國|伦敦|united ?kingdom|\buk\b|\bgb\b|london/i, "gb"],
  [/德国|德國|法兰克福|germany|\bde\b|frankfurt/i, "de"],
  [/法国|法國|巴黎|france|\bfr\b|paris/i, "fr"],
  [/俄罗斯|俄羅斯|莫斯科|russia|\bru\b|moscow/i, "ru"],
  [/印度尼西亚|印尼|indonesia|\bid\b|jakarta/i, "id"],
  [/印度|india|\bin\b|mumbai/i, "in"],
  [/加拿大|canada|\bca\b|toronto/i, "ca"],
  [/澳大利亚|澳洲|australia|\bau\b|sydney/i, "au"],
  [/新西兰|new ?zealand|\bnz\b/i, "nz"],
  [/土耳其|turkey|türkiye|\btr\b/i, "tr"],
  [/巴西|brazil|\bbr\b/i, "br"],
  [/阿根廷|argentina|\bar\b/i, "ar"],
  [/墨西哥|mexico|\bmx\b/i, "mx"],
  [/智利|chile|\bcl\b/i, "cl"],
  [/荷兰|荷蘭|阿姆斯特丹|netherlands|holland|\bnl\b|amsterdam/i, "nl"],
  [/马来西亚|馬來西亞|malaysia|\bmy\b/i, "my"],
  [/泰国|泰國|thailand|\bth\b|bangkok/i, "th"],
  [/越南|vietnam|\bvn\b/i, "vn"],
  [/菲律宾|菲律賓|philippines|\bph\b/i, "ph"],
  [/阿联酋|迪拜|dubai|emirates|\bae\b/i, "ae"],
  [/沙特|saudi|\bsa\b/i, "sa"],
  [/以色列|israel|\bil\b/i, "il"],
  [/意大利|italy|\bit\b|milan/i, "it"],
  [/西班牙|spain|\bes\b|madrid/i, "es"],
  [/葡萄牙|portugal|\bpt\b/i, "pt"],
  [/瑞士|switzerland|\bch\b|zurich/i, "ch"],
  [/瑞典|sweden|\bse\b/i, "se"],
  [/芬兰|finland|\bfi\b|helsinki/i, "fi"],
  [/挪威|norway|\bno\b/i, "no"],
  [/丹麦|denmark|\bdk\b/i, "dk"],
  [/波兰|poland|\bpl\b/i, "pl"],
  [/乌克兰|ukraine|\bua\b/i, "ua"],
  [/奥地利|austria|\bat\b/i, "at"],
  [/比利时|belgium|\bbe\b/i, "be"],
  [/爱尔兰|ireland|\bie\b|dublin/i, "ie"],
  [/捷克|czech|\bcz\b/i, "cz"],
  [/匈牙利|hungary|\bhu\b/i, "hu"],
  [/罗马尼亚|romania|\bro\b/i, "ro"],
  [/希腊|greece|\bgr\b/i, "gr"],
  [/南非|south ?africa|\bza\b/i, "za"],
  [/埃及|egypt|\beg\b/i, "eg"],
  [/尼日利亚|nigeria|\bng\b/i, "ng"],
  [/哈萨克|kazakhstan|\bkz\b/i, "kz"],
  [/卢森堡|luxembourg|\blu\b/i, "lu"],
  [/冰岛|iceland|\bis\b/i, "is"],
  [/中国|中國|回国|回國|北京|上海|广州|深圳|china|\bcn\b/i, "cn"],
];

const cache = new Map<string, string | null>();

/** 返回 flag-icons 的国家代码；识别不到返回 null */
export function nodeFlag(name: string): string | null {
  if (!name) return null;
  const hit = cache.get(name);
  if (hit !== undefined) return hit;
  let code = fromEmoji(name);
  if (!code) {
    for (const [re, c] of RULES) {
      if (re.test(name)) {
        code = c;
        break;
      }
    }
  }
  cache.set(name, code);
  return code;
}
