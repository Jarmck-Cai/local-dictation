import assert from "node:assert/strict";
import { formatKeyCombination, getKeyName } from "./keyboard";

const keyboardEvent = (value: { code?: string; key?: string }): KeyboardEvent =>
  value as KeyboardEvent;

const compoundKeys = [
  ["ScrollLock", "scrolllock", "Scroll Lock"],
  ["CapsLock", "capslock", "Caps Lock"],
  ["NumLock", "numlock", "Num Lock"],
  ["PageUp", "pageup", "Page Up"],
  ["PageDown", "pagedown", "Page Down"],
  ["PrintScreen", "printscreen", "Print Screen"],
] as const;

for (const [code, stored, displayed] of compoundKeys) {
  assert.equal(getKeyName(keyboardEvent({ code })), stored);
  assert.equal(formatKeyCombination(stored, "linux"), displayed);
}

assert.equal(getKeyName(keyboardEvent({ key: "CapsLock" })), "capslock");
assert.equal(
  formatKeyCombination("alt_left+super_left", "windows"),
  "Left Alt + Left Win",
);
assert.equal(formatKeyCombination("super_right+v", "windows"), "Right Win + V");
assert.equal(formatKeyCombination("super_left", "linux"), "Left Super");
assert.equal(
  getKeyName(keyboardEvent({ code: "AudioVolumeUp" })),
  "audiovolumeup",
);

console.log("keyboard: all assertions passed");
