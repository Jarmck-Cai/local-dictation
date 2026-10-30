import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Plus, RefreshCw } from "lucide-react";
import {
  MEETING_UPDATED_EVENT,
  meetingsApi,
  type MeetingSummary,
} from "../../lib/meetings";
import { Button } from "../ui/Button";
import { emptyMeetingsSnapshot, MeetingsState } from "./meetingState";
import { MeetingErrorMessage } from "./meetingPresentation";
import { MeetingActivity } from "./MeetingActivity";
import { MeetingDetail } from "./MeetingDetail";
import { MeetingLibrary } from "./MeetingLibrary";
import { MeetingRecordingDialog } from "./MeetingRecordingDialog";
import "./meetings.css";
export { formatMeetingTime } from "./meetingPresentation";

export const Meetings = () => {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState(emptyMeetingsSnapshot);
  const stateRef = useRef<MeetingsState | null>(null);
  const [newRecordingOpen, setNewRecordingOpen] = useState(false);
  useEffect(() => {
    const state = new MeetingsState(meetingsApi, setSnapshot);
    stateRef.current = state;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void listen<MeetingSummary>(MEETING_UPDATED_EVENT, ({ payload }) => {
      state.receive(payload);
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {
        /* Active polling and quiet reconciliation remain available. */
      });
    void state.load();
    const reconcile = setInterval(
      () => void state.refreshList({ background: true }),
      15000,
    );
    return () => {
      disposed = true;
      clearInterval(reconcile);
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
  const state = stateRef.current;
  const selected = snapshot.meetings.find(
    (row) => row.id === snapshot.selectedId,
  );
  const recordingBlocked =
    !!snapshot.busy ||
    !!snapshot.active ||
    snapshot.currentLoading ||
    !!snapshot.currentError;
  return (
    <div className="qs-page qs-meetings qs-meeting-workspace">
      <header className="qs-meeting-heading">
        <div className="qs-meeting-toolbar">
          <h1>
            {t("meetings.title")}
            <span className="qs-meeting-beta">{t("meetings.beta")}</span>
          </h1>
          <div className="qs-meeting-heading-actions">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={snapshot.listLoading}
              onClick={() => void state?.refreshList()}
            >
              <RefreshCw size={13} aria-hidden="true" />
              {t("meetings.refresh")}
            </Button>
            <Button
              type="button"
              variant="primary-soft"
              size="sm"
              disabled={recordingBlocked}
              onClick={() => setNewRecordingOpen(true)}
            >
              <Plus size={14} aria-hidden="true" />
              {t("meetings.newMeeting")}
            </Button>
          </div>
        </div>
        <details className="qs-meeting-beta-info">
          <summary>{t("meetings.description")}</summary>
          <p>{t("meetings.betaHint")}</p>
        </details>
      </header>
      <MeetingActivity snapshot={snapshot} state={state} />
      {snapshot.currentLoading && (
        <p role="status" className="qs-meeting-caption">
          {t("meetings.checkingCurrent")}
        </p>
      )}
      {snapshot.currentError && (
        <MeetingErrorMessage
          error={snapshot.currentError}
          retry={() => void state?.loadCurrent()}
        />
      )}
      {!newRecordingOpen && snapshot.error && (
        <MeetingErrorMessage error={snapshot.error} />
      )}
      {snapshot.busy === "finish" && !snapshot.active && (
        <p role="status" className="qs-meeting-caption">
          {t("meetings.finishing")}
        </p>
      )}
      {snapshot.listError && (
        <MeetingErrorMessage
          error={snapshot.listError}
          retry={() => void state?.refreshList()}
        />
      )}
      <MeetingLibrary visible={!selected} snapshot={snapshot} state={state} />
      {selected && (
        <MeetingDetail
          key={selected.id}
          selected={selected}
          snapshot={snapshot}
          state={state}
          onBack={() => state?.clearSelection()}
        />
      )}
      <p className="qs-meeting-caption qs-meeting-retention">
        {t("meetings.retention")}
      </p>
      <MeetingRecordingDialog
        open={newRecordingOpen}
        onClose={() => setNewRecordingOpen(false)}
        snapshot={snapshot}
        state={state}
      />
    </div>
  );
};
