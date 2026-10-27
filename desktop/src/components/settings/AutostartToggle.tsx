import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface AutostartToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const AutostartToggle: React.FC<AutostartToggleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { refreshSettings } = useSettings();
    const [enabled, setEnabled] = useState(false);
    const [busy, setBusy] = useState(true);
    const [failed, setFailed] = useState(false);
    const generation = useRef(0);
    const mounted = useRef(false);
    useEffect(() => {
      mounted.current = true;
      const request = ++generation.current;
      void invoke<boolean>("get_autostart_status")
        .then((value) => {
          if (mounted.current && request === generation.current) {
            setEnabled(value);
            setFailed(false);
          }
        })
        .catch(() => {
          if (mounted.current && request === generation.current)
            setFailed(true);
        })
        .finally(() => {
          if (mounted.current && request === generation.current) setBusy(false);
        });
      return () => {
        mounted.current = false;
        ++generation.current;
      };
    }, []);

    const change = async (value: boolean) => {
      if (busy) return;
      const request = ++generation.current;
      setBusy(true);
      try {
        await invoke<void>("change_autostart_setting", { enabled: value });
        const actual = await invoke<boolean>("get_autostart_status");
        if (mounted.current && request === generation.current) {
          setEnabled(actual);
          setFailed(false);
          await refreshSettings();
        }
      } catch {
        if (mounted.current && request === generation.current) {
          setFailed(true);
          toast.error(t("dictation.startup.failed"));
        }
      } finally {
        if (mounted.current && request === generation.current) setBusy(false);
      }
    };

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={change}
        isUpdating={busy}
        label={t("dictation.startup.label")}
        description={t(
          failed ? "dictation.startup.failed" : "dictation.startup.description",
        )}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
