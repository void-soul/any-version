import { describe, expect, it } from "vitest";
import { MAX_IMAGE_BASE64_CHARS, parseImageBase64 } from "./imageBase64Data";
describe("image Base64 boundary", () => {
  it("normalizes whitespace and unpadded data with exact decoded size", () => {
    expect(parseImageBase64("T Q")).toMatchObject({ src: "data:image/png;base64,TQ==", bytes: 1 });
    expect(parseImageBase64("data:image/svg+xml;base64,PHN2Zz4=")).toMatchObject({ mime: "image/svg+xml", extension: "svg", bytes: 5 });
  });
  it("rejects misplaced/excess padding and nonzero trailing bits", () => {
    for (const data of ["=", "A", "A===", "T=Q=", "TQ=", "TQ===", "TR==", "TWF=", "data:text/html;base64,TQ==", "data:image/png,TQ=="]) expect(() => parseImageBase64(data), data).toThrow("invalid");
  });
  it("recognizes raw JPEG and bounds input before parsing", () => {
    expect(parseImageBase64("/9j/")).toMatchObject({ mime: "image/jpeg", extension: "jpg", bytes: 3 });
    expect(() => parseImageBase64("A".repeat(MAX_IMAGE_BASE64_CHARS + 1))).toThrow("tooLarge");
  });
});
