import { useTranslation } from "react-i18next";
import type { MeetingSummary } from "../../lib/meetings";
import { meetingCaptureReason } from "./meetingState";

export function MeetingCaptureNotice({ meeting }: { meeting: MeetingSummary }) {
  const { t } = useTranslation();
  const cause = meetingCaptureReason(meeting);
  if (!cause) return null;
  return (
    <div className="qs-meeting-caption" role="status">
      <p>{t(`meetings.captureReasons.${cause.reason}`)}</p>
      <p>{t("meetings.warningIncompleteRecording")}</p>
      {cause.code && (
        <p>{t("meetings.captureDiagnostic", { code: cause.code })}</p>
      )}
    </div>
  );
}
