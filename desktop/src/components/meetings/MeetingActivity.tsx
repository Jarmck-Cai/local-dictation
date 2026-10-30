import { useTranslation } from "react-i18next";
import { Square } from "lucide-react";
import { Button } from "../ui/Button";
import { formatMeetingTime, useMeetingLabels } from "./meetingPresentation";
import type { MeetingsSnapshot, MeetingsState } from "./meetingState";

export function MeetingActivity({
  snapshot,
  state,
}: {
  snapshot: MeetingsSnapshot;
  state: MeetingsState | null;
}) {
  const { t } = useTranslation();
  const { meetingTitle } = useMeetingLabels();
  const active = snapshot.active;
  if (!active) return null;
  const blocked = !!snapshot.busy;
  const cancellationPending = snapshot.cancellingId === active.id;
  return (
    <section
      className="qs-meeting-card qs-meeting-active"
      aria-label={t("meetings.current")}
    >
      <div className="qs-meeting-activity-line">
        <div className="qs-meeting-activity-copy">
          <span className="qs-meeting-status" role="status">
            {t(`meetings.status.${active.status}`)}
          </span>
          <button
            type="button"
            className="qs-meeting-current-link"
            onClick={() => void state?.select(active.id)}
          >
            {meetingTitle(active)}
          </button>
        </div>
        <time className="qs-meeting-duration">
          {formatMeetingTime(active.duration_ms)}
        </time>
        <div className="qs-meeting-actions">
          {active.status === "processing" ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={blocked || cancellationPending}
              onClick={() => void state?.cancel(active.id)}
            >
              {t(
                cancellationPending || snapshot.busy === "cancel"
                  ? "meetings.cancelling"
                  : "meetings.cancelTranscription",
              )}
            </Button>
          ) : (
            <Button
              type="button"
              variant="primary-soft"
              size="sm"
              disabled={blocked}
              onClick={() => void state?.finish(active.id)}
            >
              <Square size={13} aria-hidden="true" />
              {t(
                snapshot.busy === "finish"
                  ? "meetings.finishing"
                  : "meetings.finish",
              )}
            </Button>
          )}
        </div>
      </div>
      {active.status === "processing" && (
        <div className="qs-meeting-progress">
          <progress
            aria-label={t("meetings.progressLabel")}
            max={100}
            value={active.progress_percent ?? undefined}
          />
          {active.progress_percent !== null && (
            <span>
              {t("meetings.progress", { percent: active.progress_percent })}
            </span>
          )}
        </div>
      )}
      <p className="qs-meeting-caption">
        {t(
          cancellationPending
            ? "meetings.cancelHint"
            : active.status === "processing"
              ? "meetings.processingHint"
              : "meetings.recordingHint",
        )}
      </p>
    </section>
  );
}
