import type { MeetingDevice } from "../../lib/meetings";

export function resolveMeetingDeviceId(
  devices: readonly MeetingDevice[],
  selectedId: string,
): string {
  return devices.some((device) => device.id === selectedId)
    ? selectedId
    : (devices[0]?.id ?? "");
}
