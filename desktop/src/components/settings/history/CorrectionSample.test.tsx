import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { CorrectionSample } from "./CorrectionSample";
import en from "../../../i18n/locales/en/translation.json";

const i18n = createInstance();
await i18n.init({
  lng: "en",
  resources: { en: { translation: en } },
  interpolation: { escapeValue: false },
});
const draft = {
  text: "synthetic corrected text",
  lastSubmitted: "synthetic corrected text",
  review: {
    candidates: [
      {
        id: 91,
        term: "SyntheticTerm",
        replacement_from: null,
        context: null,
        source: "correction",
        occurrences: 1,
      },
    ],
    entries: [],
  },
};
const render = () =>
  renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <CorrectionSample
        historyId={1}
        draft={draft}
        onTextChange={() => {}}
        onSubmitted={() => {}}
        onSubmittingChange={() => {}}
        onReviewChange={() => {}}
        onReviewRefresh={() => {}}
      />
    </I18nextProvider>,
  );
// Each server render creates a fresh component instance, as page switches do.
assert.ok(
  render().includes("SyntheticTerm"),
  "a remounted correction form must retain the candidate confirmation path",
);
assert.ok(render().includes("SyntheticTerm"));
console.log(
  "correction sample: remounted submitted draft retains candidate confirmation UI",
);
