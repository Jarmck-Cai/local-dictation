import { useTranslation } from "react-i18next";
import {
  BookOpen,
  ChartNoAxesCombined,
  NotebookPen,
  Settings2,
} from "lucide-react";
import { GeneralSettings } from "./settings/general/GeneralSettings";
import { HistorySettings } from "./settings/history/HistorySettings";
import { CustomWords } from "./settings/CustomWords";
import { SettingsGroup } from "./ui/SettingsGroup";
import { VocabularyLearning } from "./settings/VocabularyLearning";
import { UsageSettings } from "./settings/UsageOverview";
import { QingshuLogo } from "./brand/QingshuLogo";

const VocabularySettings = () => {
  const { t } = useTranslation();
  return (
    <div className="qs-page space-y-6">
      <VocabularyLearning />
      <SettingsGroup
        title={t("dictation.vocabulary.title")}
        description={t("dictation.vocabulary.description")}
      >
        <CustomWords descriptionMode="inline" grouped />
      </SettingsGroup>
    </div>
  );
};

export const SECTIONS_CONFIG = {
  history: {
    labelKey: "dictation.nav.history",
    icon: NotebookPen,
    component: HistorySettings,
  },
  vocabulary: {
    labelKey: "dictation.nav.vocabulary",
    icon: BookOpen,
    component: VocabularySettings,
  },
  usage: {
    labelKey: "dictation.nav.usage",
    icon: ChartNoAxesCombined,
    component: UsageSettings,
  },
  general: {
    labelKey: "dictation.nav.general",
    icon: Settings2,
    component: GeneralSettings,
  },
} as const;

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

export const Sidebar = ({
  activeSection,
  onSectionChange,
}: {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}) => {
  const { t } = useTranslation();
  const item = (id: SidebarSection) => {
    const section = SECTIONS_CONFIG[id];
    const Icon = section.icon;
    return (
      <button
        type="button"
        key={id}
        aria-label={t(section.labelKey)}
        aria-current={activeSection === id ? "page" : undefined}
        className={`qs-nav ${activeSection === id ? "qs-nav-active" : ""}`}
        onClick={() => onSectionChange(id)}
      >
        <Icon size={18} aria-hidden="true" />
        <span>{t(section.labelKey)}</span>
      </button>
    );
  };
  return (
    <aside className="qs-sidebar">
      <div className="qs-brand">
        <QingshuLogo size={37} />
        <span>{t("dictation.productName")}</span>
      </div>
      <nav aria-label={t("dictation.navigation")} className="space-y-1.5">
        {(["history", "vocabulary", "usage"] as const).map(item)}
      </nav>
      <nav aria-label={t("dictation.nav.general")} className="mt-auto pt-8">
        {item("general")}
      </nav>
    </aside>
  );
};
