import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { dictationApi, type CloudStatus } from "@/lib/dictationApi";
import { cloudState, SnapshotGuard } from "@/lib/dictationState";
import { Button } from "../ui/Button";

interface StatusSnapshot {
  status: CloudStatus;
  receivedAt: number;
}

export const DictationCloudControl = () => {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<StatusSnapshot | null>(null);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const [now, setNow] = useState(Date.now());
  const guardRef = useRef<SnapshotGuard>();

  useEffect(() => {
    const guard = new SnapshotGuard();
    guardRef.current = guard;
    let unlisten: UnlistenFn | undefined;
    let request: number | undefined;
    void (async () => {
      try {
        unlisten = await listen<CloudStatus>(
          "dictation-cloud-state",
          ({ payload }) => {
            guard.invalidate();
            if (!guard.isAlive()) return;
            setSnapshot({ status: payload, receivedAt: Date.now() });
            setFailed(false);
          },
        );
        request = guard.begin();
        if (!guard.isCurrent(request)) {
          unlisten();
          return;
        }
        const status = await dictationApi.getCloudStatus();
        if (guard.isCurrent(request)) {
          setSnapshot({ status, receivedAt: Date.now() });
          setFailed(false);
        }
      } catch {
        if (
          guard.isAlive() &&
          (request === undefined || guard.isCurrent(request))
        )
          setFailed(true);
      }
    })();
    return () => {
      guard.dispose();
      unlisten?.();
    };
  }, [reload]);

  useEffect(() => {
    setNow(Date.now());
    if (
      !snapshot ||
      snapshot.status.manual_paused ||
      snapshot.status.cooldown_remaining_ms === 0
    )
      return;
    const expiresAt =
      snapshot.receivedAt + snapshot.status.cooldown_remaining_ms;
    const timer = setInterval(() => {
      setNow(Date.now());
      if (Date.now() >= expiresAt) clearInterval(timer);
    }, 1000);
    return () => clearInterval(timer);
  }, [snapshot]);

  const toggle = async () => {
    const guard = guardRef.current;
    if (!guard || !snapshot) return;
    const request = guard.begin();
    setBusy(true);
    try {
      const status = await dictationApi.toggleCloud();
      if (guard.isCurrent(request))
        setSnapshot({ status, receivedAt: Date.now() });
    } catch {
      if (guard.isCurrent(request))
        toast.error(t("dictation.cloud.changeFailed"));
    } finally {
      // An event can supersede a mutation response, but the mounted view must unlock.
      if (guardRef.current === guard && guard.isAlive()) setBusy(false);
    }
  };

  const state = snapshot
    ? cloudState(snapshot.status, snapshot.receivedAt, now)
    : null;
  const seconds = snapshot
    ? Math.max(
        0,
        Math.ceil(
          (snapshot.status.cooldown_remaining_ms -
            (now - snapshot.receivedAt)) /
            1000,
        ),
      )
    : 0;
  return (
    <div className="p-3 rounded-lg border border-mid-gray/20 space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p role="status" aria-live="polite" className="text-sm">
          {failed
            ? t("dictation.cloud.loadFailed")
            : state
              ? t(`dictation.cloud.states.${state}`, { seconds })
              : t("common.loading")}
        </p>
        {failed ? (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setReload((value) => value + 1)}
          >
            {t("dictation.retry")}
          </Button>
        ) : (
          <Button
            variant="secondary"
            size="sm"
            disabled={busy || !snapshot}
            onClick={toggle}
          >
            {t(
              state === "manual" ||
                state === "cooldown" ||
                state === "nextRecording"
                ? "dictation.cloud.resume"
                : "dictation.cloud.pause",
            )}
          </Button>
        )}
      </div>
      <p className="text-xs text-text/60">{t("dictation.cloud.hint")}</p>
      <p className="text-xs text-text/50">{t("dictation.cloud.policy")}</p>
    </div>
  );
};
