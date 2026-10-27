import { useTranslation } from "react-i18next";
import { BookOpen, History, Mic } from "lucide-react";
import { GeneralSettings } from "./settings/general/GeneralSettings";
import { HistorySettings } from "./settings/history/HistorySettings";
import { CustomWords } from "./settings/CustomWords";
import { SettingsGroup } from "./ui/SettingsGroup";

const VocabularySettings = () => {
  const { t } = useTranslation();
  return (
    <div className="max-w-3xl w-full mx-auto">
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
  general: {
    labelKey: "dictation.nav.general",
    icon: Mic,
    component: GeneralSettings,
  },
  history: {
    labelKey: "dictation.nav.history",
    icon: History,
    component: HistorySettings,
  },
  vocabulary: {
    labelKey: "dictation.nav.vocabulary",
    icon: BookOpen,
    component: VocabularySettings,
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
  return (
    <aside className="w-44 shrink-0 border-e border-mid-gray/15 flex flex-col px-3 py-6 bg-mid-gray/3">
      <div className="px-2 pb-7">
        <div className="w-9 h-9 rounded-xl bg-logo-primary/25 text-background-ui flex items-center justify-center mb-3">
          <Mic size={20} aria-hidden="true" />
        </div>
        <p className="font-semibold text-base leading-snug">
          {t("dictation.productName")}
        </p>
        <p className="mt-1 text-xs text-text/45">{t("dictation.preview")}</p>
      </div>
      <nav className="space-y-1" aria-label={t("dictation.navigation")}>
        {Object.entries(SECTIONS_CONFIG).map(([id, section]) => {
          const Icon = section.icon;
          return (
            <button
              type="button"
              key={id}
              aria-current={activeSection === id ? "page" : undefined}
              className={`w-full flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm cursor-pointer transition-colors ${activeSection === id ? "bg-logo-primary/20 font-semibold text-text" : "text-text/65 hover:bg-mid-gray/10"}`}
              onClick={() => onSectionChange(id as SidebarSection)}
            >
              <Icon size={18} aria-hidden="true" />
              {t(section.labelKey)}
            </button>
          );
        })}
      </nav>
      <p className="mt-auto px-2 pt-8 text-xs text-text/40 leading-relaxed whitespace-pre-line">
        {t("dictation.sidebarNote")}
      </p>
    </aside>
  );
};
