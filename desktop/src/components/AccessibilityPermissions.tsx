import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
  requestAccessibilityPermission,
  requestMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { commands } from "@/bindings";
import { useSettingsStore } from "@/stores/settingsStore";
import { Button } from "./ui/Button";

type Permission = "accessibility" | "microphone";
type Access = "granted" | "required" | "unknown";

const permissions: readonly Permission[] = ["microphone", "accessibility"];

export default function AccessibilityPermissions() {
  const { t } = useTranslation();
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const [access, setAccess] = useState<Record<Permission, Access> | null>(null);
  const [startupFailed, setStartupFailed] = useState(false);
  const [actionFailed, setActionFailed] = useState(false);
  const [checking, setChecking] = useState(false);
  const pendingCheck = useRef(false);
  const hadMicrophone = useRef(false);

  const recheck = useCallback(async () => {
    if (pendingCheck.current) return;
    pendingCheck.current = true;
    setChecking(true);
    const [accessibility, microphone] = await Promise.allSettled([
      checkAccessibilityPermission(),
      checkMicrophonePermission(),
    ]);
    const result = (permission: PromiseSettledResult<boolean>): Access =>
      permission.status === "rejected"
        ? "unknown"
        : permission.value
          ? "granted"
          : "required";
    const checked = {
      accessibility: result(accessibility),
      microphone: result(microphone),
    };
    setAccess(checked);

    try {
      if (checked.accessibility === "granted") {
        const results = await Promise.allSettled([
          commands.initializeEnigo(),
          commands.initializeShortcuts(),
        ]);
        setStartupFailed(
          results.some(
            (call) => call.status !== "fulfilled" || call.value.status !== "ok",
          ),
        );
      } else {
        setStartupFailed(false);
      }
      if (checked.microphone === "granted" && !hadMicrophone.current) {
        void refreshAudioDevices();
      }
      hadMicrophone.current = checked.microphone === "granted";
    } finally {
      setChecking(false);
      pendingCheck.current = false;
    }
  }, [refreshAudioDevices]);

  useEffect(() => {
    void recheck();
    const onFocus = () => void recheck();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [recheck]);

  const openSettings = async (permission: Permission) => {
    setActionFailed(false);
    try {
      const result = await commands.openDictationPrivacySettings(permission);
      if (result.status !== "ok") setActionFailed(true);
    } catch {
      setActionFailed(true);
    }
  };

  const request = async (permission: Permission) => {
    setActionFailed(false);
    try {
      if (permission === "microphone") await requestMicrophonePermission();
      else await requestAccessibilityPermission();
      await recheck();
    } catch {
      setActionFailed(true);
    }
  };

  if (
    !access ||
    (!startupFailed &&
      permissions.every((permission) => access[permission] === "granted"))
  ) {
    return null;
  }

  return (
    <section
      aria-labelledby="mac-permission-title"
      className="max-w-3xl mx-auto mb-5 rounded-lg border border-warning/25 bg-warning/10 p-4 space-y-3"
    >
      <h2 id="mac-permission-title" className="text-sm font-semibold">
        {t("onboarding.permissions.title")}
      </h2>
      {permissions.map((permission) => (
        <div key={permission} className="space-y-1.5">
          <h3 className="text-sm font-medium">
            {t(
              permission === "microphone"
                ? "onboarding.permissions.microphone.title"
                : "onboarding.permissions.accessibility.title",
            )}
          </h3>
          <p className="text-xs text-text/70">
            {t(
              permission === "accessibility"
                ? "onboarding.permissions.macAccessibilityHelp"
                : "onboarding.permissions.macMicrophoneHelp",
            )}
          </p>
          {access[permission] === "granted" ? (
            <p className="text-xs">{t("onboarding.permissions.granted")}</p>
          ) : (
            <div className="flex flex-wrap gap-2">
              <Button
                variant="secondary"
                size="sm"
                disabled={checking}
                onClick={() => void request(permission)}
              >
                {t("onboarding.permissions.grant")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void openSettings(permission)}
              >
                {t("accessibility.openSettings")}
              </Button>
            </div>
          )}
        </div>
      ))}
      {permissions.some((permission) => access[permission] === "unknown") && (
        <p role="alert" className="text-xs">
          {t("onboarding.permissions.errors.checkFailed")}
        </p>
      )}
      {startupFailed && (
        <p role="alert" className="text-xs">
          {t("onboarding.permissions.macInputFailed")}
        </p>
      )}
      {actionFailed && (
        <p role="alert" className="text-xs">
          {t("onboarding.permissions.errors.requestFailed")}
        </p>
      )}
      <Button
        variant="secondary"
        size="sm"
        disabled={checking}
        onClick={() => void recheck()}
      >
        {t("onboarding.permissions.recheck")}
      </Button>
    </section>
  );
}
