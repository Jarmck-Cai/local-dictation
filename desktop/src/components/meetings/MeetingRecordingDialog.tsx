import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, RefreshCw, X } from "lucide-react";
import { Button } from "../ui/Button";
import { resolveMeetingDeviceId } from "./meetingDevices";
import { MeetingErrorMessage } from "./meetingPresentation";
import type { MeetingsSnapshot, MeetingsState } from "./meetingState";

export function MeetingRecordingDialog({
  open,
  onClose,
  snapshot,
  state,
}: {
  open: boolean;
  onClose: () => void;
  snapshot: MeetingsSnapshot;
  state: MeetingsState | null;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const [title, setTitle] = useState("");
  const [microphoneId, setMicrophoneId] = useState("");
  const [renderId, setRenderId] = useState("");
  useEffect(() => {
    if (open) dialog.current?.showModal();
    else dialog.current?.close();
  }, [open]);
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
  const recordingBlocked =
    !!snapshot.busy ||
    !!snapshot.active ||
    snapshot.currentLoading ||
    !!snapshot.currentError;
  return (
    <dialog
      ref={dialog}
      className="qs-meetings qs-meeting-dialog"
      aria-labelledby="meeting-new-title"
      onClose={onClose}
      onCancel={(event) => {
        if (snapshot.busy === "start") event.preventDefault();
      }}
    >
      <div className="qs-meeting-toolbar">
        <h2 id="meeting-new-title">{t("meetings.newMeeting")}</h2>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-label={t("meetings.close")}
          disabled={snapshot.busy === "start"}
          onClick={onClose}
        >
          <X size={16} aria-hidden="true" />
        </Button>
      </div>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        disabled={recordingBlocked || snapshot.devicesLoading}
        onClick={() => void state?.loadDevices()}
      >
        <RefreshCw size={13} aria-hidden="true" />
        {t("meetings.refreshDevices")}
      </Button>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (recordingBlocked || !deviceSelectionValid) return;
          void state
            ?.start({
              title: title.trim(),
              microphoneId: effectiveMicrophoneId,
              renderId: effectiveRenderId,
            })
            .then((started) => {
              if (started) {
                setTitle("");
                onClose();
              }
            });
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
            retry={() => void state?.loadDevices()}
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
          retry={() => void state?.loadCurrent()}
        />
      )}

      {snapshot.error && <MeetingErrorMessage error={snapshot.error} />}
    </dialog>
  );
}
