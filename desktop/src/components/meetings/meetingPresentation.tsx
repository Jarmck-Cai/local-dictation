import { useTranslation } from "react-i18next";
import type { MeetingSummary } from "../../lib/meetings";
import type { MeetingError } from "./meetingState";
import { Button } from "../ui/Button";
export function formatMeetingTime(milliseconds: number) {
  const seconds = Math.floor(Math.max(0, milliseconds) / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = String(seconds % 60).padStart(2, "0");
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${remainder}`
    : `${minutes}:${remainder}`;
}

export const MeetingErrorMessage = ({
  error,
  retry,
}: {
  error: MeetingError;
  retry?: () => void;
}) => {
  const { t } = useTranslation();
  return (
    <div className="qs-inline-alert" role="alert">
      <p>{t(`meetings.errors.${error}`)}</p>
      {retry && (
        <Button type="button" variant="secondary" size="sm" onClick={retry}>
          {t("meetings.retry")}
        </Button>
      )}
    </div>
  );
};

export function useMeetingLabels() {
  const { t, i18n } = useTranslation();
  const validDate = (meeting: MeetingSummary) =>
    meeting.created_at_ms > 0 &&
    Number.isFinite(new Date(meeting.created_at_ms).getTime());
  return {
    meetingTitle: (meeting: MeetingSummary) =>
      meeting.title ||
      (validDate(meeting)
        ? t("meetings.untitledDate", {
            date: new Date(meeting.created_at_ms).toLocaleDateString(
              i18n.language,
            ),
          })
        : t("meetings.untitled")),
    date: (meeting: MeetingSummary) =>
      validDate(meeting)
        ? new Date(meeting.created_at_ms).toLocaleString(i18n.language, {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          })
        : t("meetings.dateUnavailable"),
  };
}
