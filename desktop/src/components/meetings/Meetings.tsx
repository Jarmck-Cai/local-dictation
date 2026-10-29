import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, Mic, RefreshCw, Square, Trash2 } from "lucide-react";
import {
  MEETING_UPDATED_EVENT,
  meetingsApi,
  type MeetingExportFormat,
  type MeetingSummary,
} from "../../lib/meetings";
import { Button } from "../ui/Button";
import {
  canTranscribeMeeting,
  emptyMeetingsSnapshot,
  isMeetingActive,
  meetingErrorKey,
  meetingWarningKeys,
  MeetingsState,
  type MeetingError,
} from "./meetingState";
import "./meetings.css";
import { resolveMeetingDeviceId } from "./meetingDevices";

export function formatMeetingTime(milliseconds: number) {
  const seconds = Math.floor(Math.max(0, milliseconds) / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = String(seconds % 60).padStart(2, "0");
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${remainder}`
    : `${minutes}:${remainder}`;
}

const MeetingErrorMessage = ({
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

export const Meetings = () => {
  const { t, i18n } = useTranslation();
  const [snapshot, setSnapshot] = useState(emptyMeetingsSnapshot);
  const stateRef = useRef<MeetingsState | null>(null);
  const [title, setTitle] = useState("");
  const [microphoneId, setMicrophoneId] = useState("");
  const [renderId, setRenderId] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  useEffect(() => {
    const state = new MeetingsState(meetingsApi, setSnapshot);
    stateRef.current = state;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void listen<MeetingSummary>(MEETING_UPDATED_EVENT, ({ payload }) => {
      state.receive(payload);
      void state.refreshList();
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {
        // Active status polling and manual refresh remain available.
      });
    void state.load();
    return () => {
      disposed = true;
      unlisten?.();
      state.dispose();
      if (stateRef.current === state) stateRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!snapshot.active) return;
    const timer = setInterval(() => void stateRef.current?.pollCurrent(), 1000);
    return () => clearInterval(timer);
  }, [snapshot.active?.id]);

  useEffect(() => setConfirmDeleteId(null), [snapshot.selectedId]);

  const perform = async (
    operation: (state: MeetingsState) => Promise<void>,
  ) => {
    const state = stateRef.current;
    if (!state) return;
    await operation(state);
    await state.refreshList();
  };

  const selected = snapshot.meetings.find(
    (row) => row.id === snapshot.selectedId,
  );
  const microphoneDevices = snapshot.devices.filter(
    (device) => device.source === "microphone",
  );
  const playbackDevices = snapshot.devices.filter(
    (device) => device.source === "remote",
  );
  const effectiveMicrophoneId = resolveMeetingDeviceId(
    microphoneDevices,
    microphoneId,
  );
  const effectiveRenderId = resolveMeetingDeviceId(playbackDevices, renderId);
  const deviceSelectionValid = !!effectiveMicrophoneId && !!effectiveRenderId;
  const blocked = !!snapshot.busy;
  const cancellationPending =
    !!snapshot.active && snapshot.cancellingId === snapshot.active.id;
  const recordingBlocked =
    blocked ||
    !!snapshot.active ||
    snapshot.currentLoading ||
    !!snapshot.currentError;

  const exportMeeting = (
    meeting: MeetingSummary,
    format: MeetingExportFormat,
  ) =>
    perform((state) =>
      state.export(
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
                  format === "audio"
                    ? "meetings.audioFile"
                    : "meetings.textFile",
                ),
                extensions: [format === "audio" ? "wav" : "txt"],
              },
            ],
          }),
      ),
    );

  const meetingTitle = (meeting: MeetingSummary) =>
    meeting.title || t("meetings.untitled");
  const date = (meeting: MeetingSummary) =>
    new Date(meeting.created_at_ms).toLocaleString(i18n.language, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });

  return (
    <div className="qs-page qs-meetings space-y-6">
      <header className="qs-meeting-heading">
        <h1>
          {t("meetings.title")}
          <span className="qs-meeting-beta">{t("meetings.beta")}</span>
        </h1>
        <p>{t("meetings.description")}</p>
        <p className="qs-meeting-beta-hint">{t("meetings.betaHint")}</p>
      </header>

      <section className="qs-meeting-card" aria-labelledby="meeting-new-title">
        <div className="qs-meeting-toolbar">
          <h2 id="meeting-new-title">{t("meetings.newMeeting")}</h2>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={recordingBlocked || snapshot.devicesLoading}
            onClick={() => void stateRef.current?.loadDevices()}
          >
            <RefreshCw size={13} aria-hidden="true" />
            {t("meetings.refreshDevices")}
          </Button>
        </div>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (recordingBlocked || !deviceSelectionValid) return;
            void perform((state) =>
              state.start({
                title: title.trim(),
                microphoneId: effectiveMicrophoneId,
                renderId: effectiveRenderId,
              }),
            );
          }}
        >
          <div className="qs-meeting-fields">
            <label className="qs-meeting-title-field" htmlFor="meeting-title">
              <span>{t("meetings.meetingTitle")}</span>
              <input
                id="meeting-title"
                value={title}
                maxLength={120}
                placeholder={t("meetings.titlePlaceholder")}
                disabled={recordingBlocked}
                onChange={(event) => setTitle(event.target.value)}
              />
            </label>
            <label htmlFor="meeting-microphone">
              <span id="meeting-microphone-label">
                {t("meetings.microphone")}
              </span>
              <select
                id="meeting-microphone"
                aria-labelledby="meeting-microphone-label"
                value={effectiveMicrophoneId}
                disabled={recordingBlocked || snapshot.devicesLoading}
                aria-describedby="meeting-device-hint"
                onChange={(event) => setMicrophoneId(event.target.value)}
              >
                <option value="">{t("meetings.chooseMicrophone")}</option>
                {microphoneDevices.map((device) => (
                  <option key={device.id} value={device.id}>
                    {device.is_default
                      ? t("meetings.defaultDevice", { name: device.name })
                      : device.name}
                  </option>
                ))}
              </select>
            </label>
            <label htmlFor="meeting-playback">
              <span id="meeting-playback-label">{t("meetings.playback")}</span>
              <select
                id="meeting-playback"
                aria-labelledby="meeting-playback-label"
                value={effectiveRenderId}
                disabled={recordingBlocked || snapshot.devicesLoading}
                aria-describedby="meeting-playback-hint"
                onChange={(event) => setRenderId(event.target.value)}
              >
                <option value="">{t("meetings.choosePlayback")}</option>
                {playbackDevices.map((device) => (
                  <option key={device.id} value={device.id}>
                    {device.is_default
                      ? t("meetings.defaultDevice", { name: device.name })
                      : device.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p className="qs-meeting-caption" id="meeting-playback-hint">
            {t("meetings.playbackHint")}
          </p>
          <p className="qs-meeting-caption" id="meeting-device-hint">
            {t("meetings.deviceHint")}
          </p>
          {snapshot.devicesLoading && (
            <p role="status" className="qs-meeting-caption">
              {t("meetings.loadingDevices")}
            </p>
          )}
          {snapshot.devicesError && (
            <MeetingErrorMessage
              error={snapshot.devicesError}
              retry={() => void stateRef.current?.loadDevices()}
            />
          )}
          {!snapshot.devicesLoading && !snapshot.devicesError && (
            <>
              {!microphoneDevices.length && (
                <p className="qs-meeting-caption" role="status">
                  {t("meetings.noMicrophone")}
                </p>
              )}
              {!playbackDevices.length && (
                <p className="qs-meeting-caption" role="status">
                  {t("meetings.noPlayback")}
                </p>
              )}
            </>
          )}
          <div className="qs-meeting-start-row">
            <Button
              type="submit"
              variant="primary-soft"
              disabled={
                recordingBlocked ||
                !deviceSelectionValid ||
                snapshot.devicesLoading ||
                !!snapshot.devicesError
              }
            >
              <Mic size={15} aria-hidden="true" />
              {t(
                snapshot.busy === "start"
                  ? "meetings.starting"
                  : "meetings.start",
              )}
            </Button>
            {snapshot.active && (
              <p className="qs-meeting-caption">{t("meetings.activeHint")}</p>
            )}
            {snapshot.currentLoading && (
              <p role="status" className="qs-meeting-caption">
                {t("meetings.checkingCurrent")}
              </p>
            )}
          </div>
        </form>
        {snapshot.currentError && (
          <MeetingErrorMessage
            error={snapshot.currentError}
            retry={() => void stateRef.current?.loadCurrent()}
          />
        )}
      </section>

      {snapshot.active && (
        <section
          className="qs-meeting-card qs-meeting-active"
          aria-labelledby="meeting-current-title"
        >
          <div className="qs-meeting-toolbar">
            <h2 id="meeting-current-title">{t("meetings.current")}</h2>
            <span className="qs-meeting-status" role="status">
              {t(`meetings.status.${snapshot.active.status}`)}
            </span>
          </div>
          <div className="qs-meeting-toolbar">
            <button
              type="button"
              className="qs-meeting-current-link"
              onClick={() => void stateRef.current?.select(snapshot.active!.id)}
            >
              {meetingTitle(snapshot.active)}
            </button>
            <time className="qs-meeting-duration">
              {formatMeetingTime(snapshot.active.duration_ms)}
            </time>
          </div>
          <p className="qs-meeting-caption">
            {t(
              snapshot.active.status === "processing"
                ? "meetings.processingHint"
                : "meetings.recordingHint",
            )}
          </p>
          {snapshot.active.status === "processing" && (
            <div className="qs-meeting-progress" aria-live="polite">
              <progress
                aria-label={t("meetings.progressLabel")}
                max={100}
                value={snapshot.active.progress_percent ?? undefined}
              />
              {snapshot.active.progress_percent !== null && (
                <span>
                  {t("meetings.progress", {
                    percent: snapshot.active.progress_percent,
                  })}
                </span>
              )}
            </div>
          )}
          <div className="qs-meeting-actions">
            {snapshot.active.status === "processing" ? (
              <Button
                type="button"
                variant="secondary"
                disabled={blocked || cancellationPending}
                onClick={() =>
                  void perform((state) => state.cancel(snapshot.active!.id))
                }
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
                disabled={blocked}
                onClick={() =>
                  void perform((state) => state.finish(snapshot.active!.id))
                }
              >
                <Square size={13} aria-hidden="true" />
                {t(
                  snapshot.busy === "finish"
                    ? "meetings.finishing"
                    : "meetings.finish",
                )}
              </Button>
            )}
            {cancellationPending && (
              <p className="qs-meeting-caption" role="status">
                {t("meetings.cancelHint")}
              </p>
            )}
          </div>
        </section>
      )}
      {snapshot.busy === "finish" && !snapshot.active && (
        <p role="status" className="qs-meeting-caption">
          {t("meetings.finishing")}
        </p>
      )}
      {snapshot.error && <MeetingErrorMessage error={snapshot.error} />}

      <section aria-labelledby="meeting-recent-title">
        <div className="qs-meeting-toolbar">
          <h2 id="meeting-recent-title">{t("meetings.recent")}</h2>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={snapshot.listLoading || blocked}
            onClick={() => void stateRef.current?.refreshList()}
          >
            {t("meetings.refresh")}
          </Button>
        </div>
        {snapshot.listLoading && !snapshot.meetings.length && (
          <p className="qs-empty" role="status">
            {t("meetings.loading")}
          </p>
        )}
        {snapshot.listError && (
          <MeetingErrorMessage
            error={snapshot.listError}
            retry={() => void stateRef.current?.refreshList()}
          />
        )}
        {!snapshot.listLoading &&
          !snapshot.listError &&
          !snapshot.meetings.length && (
            <p className="qs-empty">{t("meetings.empty")}</p>
          )}
        {!!snapshot.meetings.length && (
          <ul className="qs-meeting-list">
            {snapshot.meetings.map((meeting) => (
              <li key={meeting.id}>
                <button
                  type="button"
                  className="qs-meeting-row"
                  aria-pressed={snapshot.selectedId === meeting.id}
                  onClick={() => void stateRef.current?.select(meeting.id)}
                >
                  <span className="qs-meeting-row-main">
                    <strong>{meetingTitle(meeting)}</strong>
                    <time
                      dateTime={new Date(meeting.created_at_ms).toISOString()}
                    >
                      {date(meeting)}
                    </time>
                  </span>
                  <span className="qs-meeting-row-meta">
                    <span>{t(`meetings.status.${meeting.status}`)}</span>
                    <span className="qs-meeting-duration">
                      {formatMeetingTime(meeting.duration_ms)}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {selected ? (
        <section
          className="qs-meeting-card"
          aria-labelledby="meeting-detail-title"
        >
          <div className="qs-meeting-toolbar">
            <h2 id="meeting-detail-title">{meetingTitle(selected)}</h2>
            <span className="qs-meeting-status">
              {t(`meetings.status.${selected.status}`)}
            </span>
          </div>
          <p className="qs-meeting-caption">{date(selected)}</p>
          {(selected.microphone_name || selected.playback_name) && (
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
          )}
          {selected.error && selected.status !== "interrupted" && (
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
              retry={() => void stateRef.current?.select(selected.id)}
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
                onClick={() =>
                  void perform((state) => state.retry(selected.id))
                }
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
                  onClick={() =>
                    void perform((state) => state.remove(selected.id))
                  }
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
        </section>
      ) : (
        !!snapshot.meetings.length && (
          <p className="qs-meeting-caption">{t("meetings.selectHint")}</p>
        )
      )}
      <p className="qs-meeting-caption">{t("meetings.retention")}</p>
    </div>
  );
};
