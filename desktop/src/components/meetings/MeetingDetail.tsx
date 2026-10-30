import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Download, Trash2 } from "lucide-react";
import type { MeetingSummary, MeetingExportFormat } from "../../lib/meetings";
import { MeetingCaptureNotice } from "./MeetingCaptureNotice";
import { Button } from "../ui/Button";
import {
  canTranscribeMeeting,
  isMeetingActive,
  meetingErrorKey,
  meetingWarningKeys,
  type MeetingsSnapshot,
  type MeetingsState,
} from "./meetingState";
import {
  formatMeetingTime,
  MeetingErrorMessage,
  useMeetingLabels,
} from "./meetingPresentation";

export function MeetingDetail({
  selected,
  snapshot,
  state,
  onBack,
}: {
  selected: MeetingSummary;
  snapshot: MeetingsSnapshot;
  state: MeetingsState | null;
  onBack: () => void;
}) {
  const { t, i18n } = useTranslation();
  const { meetingTitle, date } = useMeetingLabels();
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  useEffect(() => setConfirmDeleteId(null), [selected.id]);
  const blocked = !!snapshot.busy;
  const exportMeeting = (
    meeting: MeetingSummary,
    format: MeetingExportFormat,
  ) =>
    state?.export(
      meeting.id,
      format,
      i18n.language.startsWith("zh") ? "zh" : "en",
      () =>
        save({
          title: t(
            format === "audio"
              ? "meetings.audioSaveTitle"
              : "meetings.textSaveTitle",
          ),
          defaultPath: `meeting-${meeting.id.replace(/[^a-zA-Z0-9_-]/g, "-")}.${format === "audio" ? "wav" : "txt"}`,
          filters: [
            {
              name: t(
                format === "audio" ? "meetings.audioFile" : "meetings.textFile",
              ),
              extensions: [format === "audio" ? "wav" : "txt"],
            },
          ],
        }),
    );

  return (
    <section
      className="qs-meeting-detail"
      aria-labelledby="meeting-detail-title"
    >
      <header className="qs-meeting-detail-header">
        <Button type="button" variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft size={14} aria-hidden="true" />
          {t("meetings.back")}
        </Button>
        <div className="qs-meeting-toolbar">
          <h2 id="meeting-detail-title">{meetingTitle(selected)}</h2>
          <span className="qs-meeting-status">
            {t(`meetings.status.${selected.status}`)}
          </span>
        </div>
        <p className="qs-meeting-caption">
          {date(selected)} · {formatMeetingTime(selected.duration_ms)}
        </p>{" "}
        <div className="qs-meeting-actions">
          {canTranscribeMeeting(selected) && (
            <Button
              type="button"
              variant="primary-soft"
              disabled={
                blocked ||
                !!snapshot.active ||
                !!snapshot.currentError ||
                snapshot.currentLoading
              }
              onClick={() => void state?.retry(selected.id)}
            >
              {t(
                snapshot.busy === "transcribe"
                  ? "meetings.transcribing"
                  : selected.status === "recorded"
                    ? "meetings.transcribe"
                    : "meetings.retryTranscription",
              )}
            </Button>
          )}
          <Button
            type="button"
            variant="secondary"
            disabled={blocked || isMeetingActive(selected)}
            onClick={() => void exportMeeting(selected, "audio")}
          >
            <Download size={13} aria-hidden="true" />
            {t("meetings.exportAudio")}
          </Button>
          <Button
            type="button"
            variant="secondary"
            disabled={
              blocked ||
              isMeetingActive(selected) ||
              !snapshot.transcript?.segments.length ||
              snapshot.transcriptLoading
            }
            onClick={() => void exportMeeting(selected, "text")}
          >
            <Download size={13} aria-hidden="true" />
            {t("meetings.exportText")}
          </Button>
          <Button
            type="button"
            variant="danger-ghost"
            disabled={blocked || isMeetingActive(selected)}
            onClick={() => setConfirmDeleteId(selected.id)}
          >
            <Trash2 size={13} aria-hidden="true" />
            {t("meetings.delete")}
          </Button>
        </div>
        {snapshot.busy === "export" && (
          <p className="qs-meeting-caption" role="status">
            {t("meetings.exporting")}
          </p>
        )}
        {snapshot.exported && (
          <p className="qs-meeting-caption" role="status">
            {t("meetings.exported")}
          </p>
        )}
        <p className="qs-meeting-caption">{t("meetings.audioExportHint")}</p>
        {confirmDeleteId === selected.id && (
          <div
            className="qs-meeting-delete-confirm"
            role="group"
            aria-label={t("meetings.delete")}
          >
            <p>{t("meetings.deleteQuestion")}</p>
            <div className="qs-meeting-actions">
              <Button
                type="button"
                variant="danger"
                disabled={blocked || isMeetingActive(selected)}
                onClick={() => void state?.remove(selected.id)}
              >
                {t(
                  snapshot.busy === "delete"
                    ? "meetings.deleting"
                    : "meetings.confirmDelete",
                )}
              </Button>
              <Button
                type="button"
                variant="secondary"
                disabled={blocked}
                onClick={() => setConfirmDeleteId(null)}
              >
                {t("meetings.keep")}
              </Button>
            </div>
          </div>
        )}
      </header>
      <article
        className="qs-meeting-reader-scroll"
        tabIndex={0}
        aria-label={t("meetings.transcript")}
      >
        {(selected.microphone_name || selected.playback_name) && (
          <details className="qs-meeting-metadata">
            <summary>{t("meetings.recordedWith")}</summary>
            <dl
              className="qs-meeting-devices"
              aria-label={t("meetings.recordedWith")}
            >
              {selected.microphone_name && (
                <div>
                  <dt>{t("meetings.source.microphone")}</dt>
                  <dd>{selected.microphone_name}</dd>
                </div>
              )}
              {selected.playback_name && (
                <div>
                  <dt>{t("meetings.source.remote")}</dt>
                  <dd>{selected.playback_name}</dd>
                </div>
              )}
            </dl>
            <p className="qs-meeting-caption">{t("meetings.retention")}</p>
          </details>
        )}
        <MeetingCaptureNotice meeting={selected} />
        {selected.error &&
          (!selected.capture_error || selected.status === "failed") && (
            <MeetingErrorMessage error={meetingErrorKey(selected.error)} />
          )}
        {canTranscribeMeeting(selected) && (
          <p className="qs-meeting-caption">{t("meetings.savedHint")}</p>
        )}
        <h3 className="qs-meeting-transcript-title">
          {t("meetings.transcript")}
        </h3>
        {snapshot.transcriptLoading && (
          <p className="qs-empty" role="status">
            {t("meetings.loadingTranscript")}
          </p>
        )}
        {snapshot.transcriptError && (
          <MeetingErrorMessage
            error={snapshot.transcriptError}
            retry={() => void state?.select(selected.id)}
          />
        )}
        {!snapshot.transcriptLoading && !snapshot.transcriptError && (
          <>
            {snapshot.transcript && !snapshot.transcript.complete && (
              <p className="qs-explanation">{t("meetings.partial")}</p>
            )}
            {meetingWarningKeys(snapshot.transcript?.warnings ?? []).map(
              (key) => (
                <p key={key} className="qs-meeting-caption">
                  {t(key)}
                </p>
              ),
            )}
            {snapshot.transcript?.segments.length ? (
              <ol className="qs-meeting-transcript">
                {snapshot.transcript.segments.map((segment) => (
                  <li key={segment.id}>
                    <div className="qs-meeting-segment-heading">
                      <span
                        className={`qs-meeting-speaker qs-meeting-speaker-${segment.source}`}
                      >
                        {t(`meetings.source.${segment.source}`)}
                      </span>
                      <time>{formatMeetingTime(segment.start_ms)}</time>
                    </div>
                    <p>{segment.text}</p>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="qs-empty">
                {t(
                  isMeetingActive(selected)
                    ? "meetings.waitingTranscript"
                    : snapshot.transcript?.complete
                      ? "meetings.noSpeech"
                      : "meetings.noTranscript",
                )}
              </p>
            )}
          </>
        )}
      </article>
    </section>
  );
}
