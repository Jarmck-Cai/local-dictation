import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { VocabularyTerm } from "@/lib/dictationApi";
import { Button } from "../ui/Button";

export interface LearnedTermEdit {
  term: string;
  aliases: string[];
  context: string | null;
}

export const LearnedTermRow = ({
  entry,
  busy,
  onSave,
  onToggle,
  onDelete,
}: {
  entry: VocabularyTerm;
  busy: boolean;
  onSave: (edit: LearnedTermEdit) => Promise<boolean>;
  onToggle: () => void;
  onDelete: () => void;
}) => {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [term, setTerm] = useState(entry.term);
  const [aliases, setAliases] = useState(entry.aliases.join("\n"));
  const [context, setContext] = useState(entry.context ?? "");
  const beginEdit = () => {
    setTerm(entry.term);
    setAliases(entry.aliases.join("\n"));
    setContext(entry.context ?? "");
    setEditing(true);
  };

  return (
    <li className="p-3 rounded-lg bg-mid-gray/5 space-y-3">
      {editing ? (
        <form
          className="space-y-3"
          onSubmit={async (event) => {
            event.preventDefault();
            if (
              await onSave({
                term,
                aliases: aliases.trim() === "" ? [] : aliases.split("\n"),
                context: context.trim() || null,
              })
            )
              setEditing(false);
          }}
        >
          <label className="block text-xs space-y-1">
            <span>{t("dictation.vocabulary.editTerm")}</span>
            <input
              className="w-full rounded border border-mid-gray/25 bg-background p-2 text-sm"
              value={term}
              disabled={busy}
              onChange={(event) => setTerm(event.target.value)}
              required
            />
          </label>
          <label className="block text-xs space-y-1">
            <span>{t("dictation.vocabulary.editAliases")}</span>
            <textarea
              className="w-full rounded border border-mid-gray/25 bg-background p-2 text-sm"
              value={aliases}
              rows={3}
              disabled={busy}
              onChange={(event) => setAliases(event.target.value)}
            />
          </label>
          <label className="block text-xs space-y-1">
            <span>{t("dictation.vocabulary.editContext")}</span>
            <input
              className="w-full rounded border border-mid-gray/25 bg-background p-2 text-sm"
              value={context}
              disabled={busy}
              onChange={(event) => setContext(event.target.value)}
            />
          </label>
          <p className="text-xs text-text/55">
            {t("dictation.vocabulary.editHint")}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" type="submit" disabled={busy}>
              {t("dictation.vocabulary.save")}
            </Button>
            <Button
              size="sm"
              type="button"
              variant="secondary"
              disabled={busy}
              onClick={() => setEditing(false)}
            >
              {t("dictation.vocabulary.cancel")}
            </Button>
          </div>
        </form>
      ) : (
        <>
          <div className="text-sm select-text break-words">
            <div className="flex flex-wrap items-center gap-2">
              <p>{entry.term}</p>
              <span className="text-xs text-text/55">
                {t(
                  entry.enabled
                    ? "dictation.vocabulary.enabled"
                    : "dictation.vocabulary.disabled",
                )}
              </span>
            </div>
            {entry.aliases.length > 0 && (
              <p className="text-xs text-text/55 mt-1">
                {t("dictation.vocabulary.aliases", {
                  aliases: entry.aliases.join(
                    t("dictation.vocabulary.separator"),
                  ),
                })}
              </p>
            )}
            {entry.context && (
              <p className="text-xs text-text/55 mt-1">
                {t("dictation.vocabulary.context", { context: entry.context })}
              </p>
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              onClick={beginEdit}
            >
              {t("dictation.vocabulary.edit")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              aria-pressed={entry.enabled}
              onClick={onToggle}
            >
              {t(
                entry.enabled
                  ? "dictation.vocabulary.disable"
                  : "dictation.vocabulary.enable",
              )}
            </Button>
            <Button
              size="sm"
              variant="danger-ghost"
              disabled={busy}
              onClick={onDelete}
            >
              {t("dictation.vocabulary.delete")}
            </Button>
          </div>
        </>
      )}
    </li>
  );
};
