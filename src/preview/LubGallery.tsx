import { StrictMode, useEffect } from "react";
import ReactDOM from "react-dom/client";
import { I18nextProvider, useTranslation } from "react-i18next";
import i18n from "@/i18n";
import { ensureProCopy } from "@/i18n/proCopy";
import { CapsLegendWindow } from "@/components/lub/LegendCard";
import { TrayPanelV5 } from "@/components/lub/TrayPanelV5";
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

function Gallery({ screen, lang }: { screen: string; lang: "zh" | "en" }) {
  const locale = lang === "en" ? "en" : "zh-CN";
  const model =
    screen === "killed"
      ? killedPanelFixture()
      : screen === "panelbeta"
        ? betaPanelFixture()
        : screen === "panelstale"
          ? designPanelFixture({ stale: true })
          : designPanelFixture();
  return (
    <GalleryReady>
      <div className="lub-gallery">
        <MenuBar model={model} locale={locale} />
        {screen === "settings" ? (
          <div className="lub-stage lub-stage-window">
            <CapsLegendWindow />
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
        {/* Canonical key lub.trends.all_data_is_sample_data lands with FE-12. */}
        <p className="lub-sample-mark">
          {lang === "en" ? "All data is sample data" : "全部为示例数据"}
        </p>
      </div>
    </GalleryReady>
  );
}

export function renderLubGallery(params: URLSearchParams) {
  const theme = params.get("theme") === "dark" ? "dark" : "light";
  const lang = params.get("lang") === "en" ? "en" : "zh";
  const screen = params.get("lub") || "panel";
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.classList.add("is-mac");
  document.documentElement.dataset.windowSurface = "lub-gallery";
  document.body.classList.add("lub-gallery");
  void i18n.changeLanguage(lang).finally(() => {
    ReactDOM.createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <I18nextProvider i18n={i18n}>
          <Gallery screen={screen} lang={lang} />
        </I18nextProvider>
      </StrictMode>,
    );
  });
}
