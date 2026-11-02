import { expect, it } from "vitest";
import { parseServicePort } from "./servicePort";
it("accepts integer TCP port boundaries without rounding or overflow", () => {
  expect(parseServicePort("1")).toBe(1);
  expect(parseServicePort(" 65535 ")).toBe(65535);
  for (const value of ["", "0", "-1", "65536", "1.5", "1e3", "NaN", "Infinity", "999999999999999999"]) {
    expect(parseServicePort(value), value).toBeNull();
  }
});
