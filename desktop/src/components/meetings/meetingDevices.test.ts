import assert from "node:assert/strict";
import type { MeetingDevice } from "../../lib/meetings";
import { resolveMeetingDeviceId } from "./meetingDevices";

const devices: MeetingDevice[] = [
  { id: "first", name: "First", source: "microphone", is_default: false },
  { id: "default", name: "Default", source: "microphone", is_default: true },
];

// Loading chooses the first device, even when another is the system default.
assert.equal(resolveMeetingDeviceId(devices, ""), "first");
// A valid manual choice survives refresh and enumeration order changes.
assert.equal(resolveMeetingDeviceId(devices, "default"), "default");
assert.equal(resolveMeetingDeviceId([...devices].reverse(), "first"), "first");
// A removed device falls back to the first remaining device.
assert.equal(resolveMeetingDeviceId(devices.slice(1), "first"), "default");
assert.equal(resolveMeetingDeviceId([], "first"), "");
assert.equal(resolveMeetingDeviceId([], ""), "");

// Microphone and playback lists resolve independently; either can be missing.
const playback: MeetingDevice[] = [
  { id: "speaker", name: "Speakers", source: "remote", is_default: false },
];
assert.equal(resolveMeetingDeviceId(playback, ""), "speaker");
assert.equal(resolveMeetingDeviceId(playback, "first"), "speaker");
assert.equal(
  Boolean(
    resolveMeetingDeviceId(devices, "") && resolveMeetingDeviceId([], ""),
  ),
  false,
);
console.log("Meeting device selection tests passed");
