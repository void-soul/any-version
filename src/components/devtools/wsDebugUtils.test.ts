import { describe, expect, it } from "vitest";
import {
  MAX_TARGETS,
  buildExportText,
  emptyTargets,
  filterLogs,
  formatDuration,
  normalizeHex,
  parseHeadersText,
  parseSseBlock,
  pushTarget,
  withLineEnding,
  type LogEntry,
} from "./wsDebugUtils";

const log = (over: Partial<LogEntry>): LogEntry => ({
  id: 1,
  dir: "rx",
  text: "",
  bytes: 0,
  time: "12:00:00",
  ...over,
});

describe("parseHeadersText", () => {
  it("解析多行请求头并保留值里的冒号", () => {
    expect(
      parseHeadersText("Authorization: Bearer a:b\n\n  X-Test :  1  \n")
    ).toEqual([
      ["Authorization", "Bearer a:b"],
      ["X-Test", "1"],
    ]);
  });

  it("丢弃没有冒号或键为空的行走", () => {
    expect(parseHeadersText("bad-line\n: no-key\nGood: v")).toEqual([["Good", "v"]]);
  });

  it("没有有效行时返回 null", () => {
    expect(parseHeadersText("   \n\n")).toBeNull();
    expect(parseHeadersText("bad")).toBeNull();
  });
});

describe("parseSseBlock", () => {
  it("解析 event / id / retry / 多行 data", () => {
    expect(
      parseSseBlock("event: message\nid: 7\ndata: hello\ndata: world\nretry: 3000")
    ).toEqual({ event: "message", id: "7", retry: 3000, data: "hello\nworld" });
  });

  it("忽略注释行与未知字段", () => {
    expect(parseSseBlock(": keep-alive\nfoo: bar\ndata: x")).toEqual({ data: "x" });
  });

  it("空块返回空对象", () => {
    expect(parseSseBlock("")).toEqual({});
    expect(parseSseBlock(": only comment\n")).toEqual({});
  });

  it("retry 非数字不写入", () => {
    expect(parseSseBlock("retry: abc\ndata: x")).toEqual({ data: "x" });
  });
});

describe("pushTarget", () => {
  it("去重并前置最近的连接目标", () => {
    let map = emptyTargets();
    map = pushTarget(map, "ws", "ws://a");
    map = pushTarget(map, "ws", "ws://b");
    map = pushTarget(map, "ws", "ws://a");
    expect(map.ws).toEqual(["ws://a", "ws://b"]);
    expect(map.tcp).toEqual([]);
  });

  it("每个协议最多保留 8 条", () => {
    let map = emptyTargets();
    for (let i = 0; i < 12; i++) map = pushTarget(map, "tcp", `127.0.0.1:${1000 + i}`);
    expect(map.tcp).toHaveLength(MAX_TARGETS);
    expect(map.tcp[0]).toBe("127.0.0.1:1011");
  });

  it("空目标不写入", () => {
    const map = emptyTargets();
    expect(pushTarget(map, "udp", "   ")).toBe(map);
  });
});

describe("filterLogs", () => {
  const logs: LogEntry[] = [
    log({ id: 1, dir: "rx", text: "Hello World" }),
    log({ id: 2, dir: "tx", text: "binary", hex: "01 A0 FF" }),
    log({ id: 3, dir: "event", text: "raw", meta: { event: "message", id: "42", data: "payload" } }),
    log({ id: 4, dir: "sys", text: "connected" }),
  ];

  it("匹配文本、HEX、SSE 元信息，忽略大小写", () => {
    expect(filterLogs(logs, "hello").map((x) => x.id)).toEqual([1]);
    expect(filterLogs(logs, "a0 ff").map((x) => x.id)).toEqual([2]);
    expect(filterLogs(logs, "42").map((x) => x.id)).toEqual([3]);
    expect(filterLogs(logs, "PAYLOAD").map((x) => x.id)).toEqual([3]);
  });

  it("空查询返回原数组", () => {
    expect(filterLogs(logs, "  ")).toBe(logs);
  });

  it("无匹配返回空数组", () => {
    expect(filterLogs(logs, "nope")).toEqual([]);
  });
});

describe("buildExportText", () => {
  it("按时间戳 / HEX 开关输出，并带上 SSE 元信息", () => {
    const logs: LogEntry[] = [
      log({ id: 1, dir: "rx", text: "hello" }),
      log({ id: 2, dir: "rx", text: "bye", hex: "62 79 65" }),
      log({ id: 3, dir: "event", text: "raw", meta: { event: "message", id: "9" } }),
    ];
    expect(buildExportText(logs, false, true)).toBe(
      [
        "[12:00:00] RX hello",
        "[12:00:00] RX bye",
        "[12:00:00] EVENT {event=message id=9} raw",
      ].join("\n")
    );
    expect(buildExportText([logs[1]], true, false)).toBe("RX 62 79 65");
  });
});

describe("formatDuration", () => {
  it("输出 HH:MM:SS", () => {
    expect(formatDuration(0)).toBe("00:00:00");
    expect(formatDuration(61_000)).toBe("00:01:01");
    expect(formatDuration(3_661_000)).toBe("01:01:01");
    expect(formatDuration(90_000_000)).toBe("25:00:00");
  });

  it("负数按 0 处理", () => {
    expect(formatDuration(-5000)).toBe("00:00:00");
  });
});

describe("withLineEnding", () => {
  it("只追加所选结束符", () => {
    expect(withLineEnding("AT", "none")).toBe("AT");
    expect(withLineEnding("AT", "lf")).toBe("AT\n");
    expect(withLineEnding("AT", "crlf")).toBe("AT\r\n");
    expect(withLineEnding("AT", "cr")).toBe("AT\r");
  });
});

describe("normalizeHex", () => {
  it("合法 HEX 规范化为大写空格分隔", () => {
    expect(normalizeHex("01,02 ff")).toBe("01 02 FF");
    expect(normalizeHex("aabb")).toBe("AA BB");
  });

  it("奇数位 / 非十六进制返回 null", () => {
    expect(normalizeHex("ABC")).toBeNull();
    expect(normalizeHex("ZZ")).toBeNull();
    expect(normalizeHex("中文")).toBeNull();
  });
});
