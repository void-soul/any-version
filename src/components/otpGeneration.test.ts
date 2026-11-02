import { expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { generateCode, otpGenerationStep, type OtpToken } from "./OtpPanel.model";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const token = {tokenType:"MOTP",secret:"1234567890abcdef",pin:"1234",period:10,digits:6,algorithm:"SHA1",counter:0} as OtpToken;
it("MOTP uses the existing native generator instead of unsupported WebCrypto MD5", async () => {
  vi.mocked(invoke).mockResolvedValueOnce("abc123");
  expect(await generateCode(token, 100000)).toBe("abc123");
  expect(invoke).toHaveBeenLastCalledWith("otp_generate_code", {token});
});
it("Yandex shares the existing native contract and native failures stay explicit", async () => {
  const yandex = {...token,tokenType:"Yandex",digits:8};
  vi.mocked(invoke).mockResolvedValueOnce("abcdefgh");
  expect(await generateCode(yandex, 100000)).toBe("abcdefgh");
  vi.mocked(invoke).mockRejectedValueOnce(Error("unavailable"));
  expect(await generateCode(yandex, 100000)).toBe("ERROR");
});

it("invalid periods never reach native generation and fixed-period types share their countdown", async () => {
  vi.mocked(invoke).mockClear();
  expect(await generateCode({...token,period:0},100000)).toBe("ERROR");
  expect(invoke).not.toHaveBeenCalled();
  expect(otpGenerationStep({...token,tokenType:"TOTP",period:30},29000)).toBe(0);
  expect(otpGenerationStep({...token,tokenType:"TOTP",period:30},30000)).toBe(1);
  expect(otpGenerationStep({...token,tokenType:"Yandex",period:10},29000)).toBe(0);
  expect(otpGenerationStep({...token,tokenType:"HOTP",counter:5},999999)).toBe(5);
});
