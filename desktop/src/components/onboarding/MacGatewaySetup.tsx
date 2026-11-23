import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { cloudGatewayCommands } from "@/lib/cloudGateway";
import { Button } from "../ui/Button";

const SETUP_SEEN = "qingshu-macos-gateway-choice";

export function MacGatewaySetup({ onConfigure }: { onConfigure: () => void }) {
  const { t } = useTranslation();
  const [show, setShow] = useState(false);

  useEffect(() => {
    try {
      if (localStorage.getItem(SETUP_SEEN)) return;
    } catch {
      // The optional entry can still be dismissed for this session.
    }
    let current = true;
    void cloudGatewayCommands.getConfig().then(
      (gateway) => {
        if (current && !gateway.gateway_ready) setShow(true);
      },
      () => {},
    );
    return () => {
      current = false;
    };
  }, []);

  const dismiss = () => {
    setShow(false);
    try {
      localStorage.setItem(SETUP_SEEN, "1");
    } catch {
      // Keep the app usable when webview storage is unavailable.
    }
  };

  if (!show) return null;
  return (
    <section
      aria-labelledby="mac-gateway-title"
      className="qs-page mb-4 rounded-lg border border-mid-gray/20 p-4 space-y-2"
    >
      <h2 id="mac-gateway-title" className="text-sm font-semibold">
        {t("dictation.cloudGateway.setupTitle")}
      </h2>
      <p className="text-xs text-text/70">
        {t("dictation.cloudGateway.setupDescription")}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="primary-soft" onClick={dismiss}>
          {t("dictation.cloudGateway.continueLocal")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            dismiss();
            onConfigure();
          }}
        >
          {t("dictation.cloudGateway.configure")}
        </Button>
      </div>
    </section>
  );
}
