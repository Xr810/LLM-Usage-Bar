import { StrictMode, useEffect } from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { I18nextProvider, useTranslation } from "react-i18next";
import i18n from "@/i18n";
import { ensureProCopy } from "@/i18n/proCopy";
import { LicenseSettings } from "@/components/lub/LicenseSettings";
import { TrayPanelV5 } from "@/components/lub/TrayPanelV5";
import { TrendsPage } from "@/components/lub/TrendsPage";
import { splitUsd } from "@/components/lub/money";
import {
  betaPanelFixture,
  designPanelFixture,
  killedPanelFixture,
} from "@/components/lub/panelFixture";
import type { LubPanelModel } from "@/components/lub/panelModel";

function GalleryReady({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    document.body.dataset.ready = "1";
  }, []);
  return children;
}

function MenuBar({ model, locale }: { model: LubPanelModel; locale: string }) {
  ensureProCopy();
  const { t } = useTranslation();
  const stopped = model.state === "killed";
  const amount = splitUsd(model.todayUsd, locale).text;
  return (
    <div className="lub-menubar">
      <span className="lub-muted">Wi-Fi</span>
      <span className="lub-tray-sample">
        <i
          className={stopped ? "lub-dot lub-dot-killed" : "lub-dot"}
          aria-hidden="true"
        />
        <span>{stopped ? t("lub.killed.stopped", { ns: "pro" }) : amount}</span>
      </span>
      <span className="lub-muted">
        {t("lub.common.sun", { time: "18:40" })}
      </span>
    </div>
  );
}

function SampleMark() {
  const { t } = useTranslation();
  return (
    <p className="lub-sample-mark">{t("lub.trends.all_data_is_sample_data")}</p>
  );
}

const galleryQueryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function Gallery({
  screen,
  lang,
  tier,
  features,
}: {
  screen: string;
  lang: "zh" | "en";
  tier: "free" | "pro";
  features: readonly string[];
}) {
  const locale = lang === "en" ? "en" : "zh-CN";
  const model =
    screen === "killed"
      ? killedPanelFixture()
      : screen === "panelbeta"
        ? betaPanelFixture()
        : screen === "lapsed"
          ? designPanelFixture({ licenseLapsed: true })
          : designPanelFixture();
  return (
    <GalleryReady>
      <div className="lub-gallery">
        <MenuBar model={model} locale={locale} />
        {screen === "license" ? (
          <div className="lub-stage lub-stage-window">
            <LicenseSettings />
          </div>
        ) : screen === "trends" ? (
          <div className="lub-stage">
            <QueryClientProvider client={galleryQueryClient}>
              <TrendsPage tier={tier} features={features} />
            </QueryClientProvider>
          </div>
        ) : (
          <div className="lub-stage">
            <TrayPanelV5
              model={model}
              locale={locale}
              openDayTip={screen === "tooltip"}
              scrollMaxPx={screen === "panelscroll" ? 214 : undefined}
              onOpenTrends={() => undefined}
              onOpenSettings={() => undefined}
            />
          </div>
        )}
        <SampleMark />
      </div>
    </GalleryReady>
  );
}

export function renderLubGallery(params: URLSearchParams) {
  const theme = params.get("theme") === "dark" ? "dark" : "light";
  const lang = params.get("lang") === "en" ? "en" : "zh";
  const screen = params.get("lub") || "panel";
  const tier = params.get("tier") === "pro" ? "pro" : "free";
  const features =
    params.get("upsell") === "1"
      ? ["upsell_entry", "heatmap_weekday_hour"]
      : [];
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.classList.add("is-mac");
  document.documentElement.dataset.windowSurface = "lub-gallery";
  document.body.classList.add("lub-gallery");
  void i18n.changeLanguage(lang).finally(() => {
    ReactDOM.createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <I18nextProvider i18n={i18n}>
          <Gallery
            screen={screen}
            lang={lang}
            tier={tier}
            features={features}
          />
        </I18nextProvider>
      </StrictMode>,
    );
  });
}
