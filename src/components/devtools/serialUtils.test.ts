import { describe, expect, it } from "vitest";
import {
  applyChecksum,
  buildExportText,
  checksumBytes,
  normalizeRules,
  parseHexBytes,
  pushHistory,
  splitSendLines,
} from "./serialUtils";

describe("serial hex helpers", () => {
  it("解析常见分隔符", () => {
    expect(parseHexBytes("01 a0 FF")).toEqual([0x01, 0xa0, 0xff]);
    expect(parseHexBytes("01,02:03-04")).toEqual([1, 2, 3, 4]);
  });

  it("拒绝奇数位与非法字符", () => {
    expect(parseHexBytes("01 A")).toBeNull();
    expect(parseHexBytes("01 中文")).toBeNull();
  });

  it("CRC16 Modbus 标准校验向量", () => {
    const bytes = Array.from("123456789", (c) => c.charCodeAt(0));
    expect(checksumBytes(bytes, "crc16")).toEqual([0x37, 0x4b]);
  });

  it("XOR / SUM 校验", () => {
    expect(checksumBytes([0x01, 0x02, 0x03], "xor")).toEqual([0x00]);
    expect(checksumBytes([0xff, 0x02], "sum")).toEqual([0x01]);
  });

  it("追加校验后保持 HEX 大写格式", () => {
    const res = applyChecksum("01 02 03", "crc16");
    expect(res?.text).toBe("01 02 03 61 61");
    expect(res?.checksumHex).toBe("61 61");
    expect(applyChecksum("01 02", "none")).toBeNull();
    expect(applyChecksum("zz", "xor")).toBeNull();
  });
});

describe("serial send lines", () => {
  it("按 LF / CRLF 切行并去掉空行", () => {
    expect(splitSendLines("AT\r\nAT+CSQ\n\nAT+CGMR\r")).toEqual(["AT", "AT+CSQ", "AT+CGMR"]);
  });
});

describe("serial rules", () => {
  it("导入时纠正未知匹配方式并限制延迟", () => {
    const rules = normalizeRules([
      { pattern: "AT", match_type: "unknown", delay_ms: 999999 },
      { pattern: "01", pattern_hex: true, match_type: "prefix", response: "OK" },
    ]);
    expect(rules[0].match_type).toBe("contains");
    expect(rules[0].delay_ms).toBe(10000);
    expect(rules[1].pattern_hex).toBe(true);
  });

  it("拒绝非数组与空数组", () => {
    expect(() => normalizeRules({})).toThrow();
    expect(() => normalizeRules([])).toThrow();
  });
});

describe("serial history / export", () => {
  it("历史去重且最新的排在最前", () => {
    expect(pushHistory(["B", "A"], "A")).toEqual(["A", "B"]);
    expect(pushHistory([], "  ")).toEqual([]);
  });

  it("导出文本包含方向与 HEX 视图", () => {
    const logs = [
      { id: 1, dir: "rx" as const, text: "OK", hex: "4F 4B", bytes: 2, time: "12:00:00.000" },
      { id: 2, dir: "sys" as const, text: "已连接", bytes: 0, time: "12:00:01.000" },
    ];
    expect(buildExportText(logs, true, true)).toBe("[12:00:00.000] RX 4F 4B\n[12:00:01.000] SYS 已连接");
    expect(buildExportText(logs, false, false)).toBe("RX OK\nSYS 已连接");
  });
});
