import { describe, expect, it } from "vitest";
import {
  comboFromKeyboardEvent,
  isGlobalSafeCombo,
  isValidCombo,
  normalizeCombo,
} from "../hotkey";

function key(init: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "",
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...init,
  } as KeyboardEvent;
}

describe("launcher hotkey utils", () => {
  it("records modifier + key in a stable order", () => {
    expect(comboFromKeyboardEvent(key({ key: "l", ctrlKey: true, shiftKey: true }))).toBe(
      "Ctrl+Shift+L"
    );
    expect(comboFromKeyboardEvent(key({ key: " ", altKey: true }))).toBe("Alt+Space");
    expect(comboFromKeyboardEvent(key({ key: "F2" }))).toBe("F2");
  });

  it("ignores pure modifier presses and unsupported characters", () => {
    expect(comboFromKeyboardEvent(key({ key: "Control", ctrlKey: true }))).toBeNull();
    expect(comboFromKeyboardEvent(key({ key: "中" }))).toBeNull();
  });

  it("normalizes differently-ordered input", () => {
    expect(normalizeCombo("shift+ctrl+l")).toBe("Ctrl+Shift+L");
    expect(normalizeCombo("ALT + space")).toBe("Alt+Space");
    expect(normalizeCombo("")).toBe("");
  });

  it("flags global-safe and valid combinations", () => {
    expect(isGlobalSafeCombo("Ctrl+Shift+L")).toBe(true);
    expect(isGlobalSafeCombo("Win+V")).toBe(true);
    expect(isGlobalSafeCombo("Shift+L")).toBe(false);
    expect(isValidCombo("Ctrl+Shift+L")).toBe(true);
    expect(isValidCombo("F12")).toBe(true);
    expect(isValidCombo("Ctrl+中")).toBe(false);
  });
});
