import { useTranslation } from "react-i18next";
import { CommunityBuildBadge } from "@/components/lub/CommunityBuildBadge";
import {
  communityChromeVisible,
  useLubEdition,
} from "@/components/lub/detectLubBuild";
import type { LubBuild } from "@/components/lub/panelModel";
import { SettingsSection } from "./SettingsSection";

/** About shows the community badge only. Official Free and Pro render nothing. */
export function AboutSection({ build }: { build?: LubBuild }) {
  const detected = useLubEdition();
  const resolved = build ?? detected.build;
  const { t } = useTranslation();
  if (
    !communityChromeVisible({
      build: resolved,
      upsellEntry: detected.upsellEntry,
    })
  ) {
    return null;
  }
  return (
    <SettingsSection title={t("lub.settings.about")}>
      <CommunityBuildBadge />
    </SettingsSection>
  );
}
