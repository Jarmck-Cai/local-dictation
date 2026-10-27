import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { ShortcutInput } from "./ShortcutInput";

export function ManualPasteShortcut() {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const bindings = getSetting("bindings");
  const [conflict, setConflict] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let disposed = false;
    void invoke<boolean>("get_manual_paste_binding_conflict")
      .then((value) => {
        if (!disposed) {
          setConflict(value);
          setFailed(false);
        }
      })
      .catch(() => {
        if (!disposed) setFailed(true);
      });
    return () => {
      disposed = true;
    };
  }, [bindings]);
  return (
    <>
      <ShortcutInput
        shortcutId="paste_latest_result"
        descriptionMode="inline"
        grouped
      />
      {(conflict || failed) && (
        <p role="alert" className="px-4 pb-3 text-xs text-error">
          {t(
            conflict
              ? "dictation.manualPaste.binding_conflict"
              : "dictation.manualPaste.bindingCheckFailed",
          )}
        </p>
      )}
    </>
  );
}
