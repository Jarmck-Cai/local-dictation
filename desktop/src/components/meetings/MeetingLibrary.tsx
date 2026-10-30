import { useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, FileText } from "lucide-react";
import { Button } from "../ui/Button";
import { formatMeetingTime, useMeetingLabels } from "./meetingPresentation";
import type { MeetingsSnapshot, MeetingsState } from "./meetingState";
type LibraryFilter = "all" | "complete" | "pending";
export function MeetingLibrary({
  visible,
  snapshot,
  state,
}: {
  visible: boolean;
  snapshot: MeetingsSnapshot;
  state: MeetingsState | null;
}) {
  const { t } = useTranslation();
  const { meetingTitle, date } = useMeetingLabels();
  const [filter, setFilter] = useState<LibraryFilter>("all");
  const scroll = useRef<HTMLDivElement>(null);
  const position = useRef(0);
  const openedRow = useRef<HTMLButtonElement | null>(null);
  useLayoutEffect(() => {
    if (visible && scroll.current) {
      openedRow.current?.focus({ preventScroll: true });
      scroll.current.scrollTop = position.current;
    }
  }, [visible]);
  const counts = {
    all: snapshot.meetings.length,
    complete: snapshot.meetings.filter((row) => row.status === "complete")
      .length,
    pending: snapshot.meetings.filter((row) => row.status !== "complete")
      .length,
  };
  const shown = snapshot.meetings.filter(
    (row) =>
      filter === "all" ||
      (filter === "complete"
        ? row.status === "complete"
        : row.status !== "complete"),
  );
  return (
    <section
      hidden={!visible}
      className="qs-meeting-library"
      aria-labelledby="meeting-recent-title"
    >
      <div className="qs-meeting-library-head">
        <h2 id="meeting-recent-title">{t("meetings.recent")}</h2>
        <div
          className="qs-meeting-filters"
          role="group"
          aria-label={t("meetings.filterLabel")}
        >
          {(["all", "complete", "pending"] as const).map((value) => (
            <Button
              key={value}
              type="button"
              size="sm"
              variant="ghost"
              aria-pressed={filter === value}
              onClick={() => {
                position.current = 0;
                if (scroll.current) scroll.current.scrollTop = 0;
                setFilter(value);
              }}
            >
              {t(`meetings.filters.${value}`)}
              <span>{counts[value]}</span>
            </Button>
          ))}
        </div>
      </div>
      <div
        className="qs-meeting-library-scroll"
        ref={scroll}
        onScroll={() => {
          if (visible && scroll.current)
            position.current = scroll.current.scrollTop;
        }}
      >
        {snapshot.listLoading && !snapshot.meetings.length && (
          <p className="qs-empty" role="status">
            {t("meetings.loading")}
          </p>
        )}
        {!snapshot.listLoading &&
          !snapshot.listError &&
          !snapshot.meetings.length && (
            <p className="qs-empty">{t("meetings.empty")}</p>
          )}
        {!!snapshot.meetings.length && !shown.length && (
          <p className="qs-empty">{t("meetings.emptyFilter")}</p>
        )}
        <ul className="qs-meeting-list">
          {shown.map((meeting) => (
            <li key={meeting.id}>
              <button
                type="button"
                className="qs-meeting-row"
                onClick={(event) => {
                  openedRow.current = event.currentTarget;
                  void state?.select(meeting.id);
                }}
              >
                <FileText
                  size={19}
                  className="qs-meeting-row-icon"
                  aria-hidden="true"
                />
                <span className="qs-meeting-row-main">
                  <strong>{meetingTitle(meeting)}</strong>
                  <span>
                    {date(meeting)} · {formatMeetingTime(meeting.duration_ms)}
                  </span>
                </span>
                <span className="qs-meeting-row-meta">
                  {t(`meetings.status.${meeting.status}`)}
                </span>
                <ChevronRight size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
