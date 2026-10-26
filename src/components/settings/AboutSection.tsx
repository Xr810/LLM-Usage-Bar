import { useTranslation } from "react-i18next";
import { CommunityBuildBadge } from "@/components/lub/CommunityBuildBadge";
import {
  communityChromeVisible,
  detectLubBuild,
} from "@/components/lub/detectLubBuild";
import type { LubBuild } from "@/components/lub/panelModel";
import { SettingsSection } from "./SettingsSection";

/** About shows the community badge only. Official Free and Pro render nothing. */
export function AboutSection({
  build = detectLubBuild(),
}: {
  build?: LubBuild;
}) {
  const { t } = useTranslation();
  if (!communityChromeVisible({ build })) return null;
  return (
    <SettingsSection title={t("lub.settings.about")}>
      <CommunityBuildBadge />
    </SettingsSection>
  );
}
