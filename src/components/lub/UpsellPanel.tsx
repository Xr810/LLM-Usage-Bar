import { useTranslation } from "react-i18next";

/**
 * Phase-2 skeleton. Batch 1 never mounts it. The trial and compare actions
 * stay on the panel until the compare page exists; they do not navigate.
 */
export function UpsellPanel({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  return (
    <div
      className="lub-upsell"
      role="dialog"
      aria-labelledby="lub-upsell-title"
      data-testid="lub-upsell"
    >
      <h2 id="lub-upsell-title" className="lub-title">
        {t("lub.upsell.title", { ns: "pro" })}
      </h2>
      <p>{t("lub.upsell.body", { ns: "pro" })}</p>
      <div className="lub-upsell-preview" aria-hidden="true">
        <span style={{ height: "46%" }} />
        <span style={{ height: "72%" }} />
        <span style={{ height: "38%" }} />
        <span style={{ height: "64%" }} />
      </div>
      <p className="lub-muted">{t("lub.upsell.preview_note", { ns: "pro" })}</p>
      <div className="lub-upsell-actions">
        <button
          type="button"
          className="lub-upsell-primary"
          onClick={(event) => event.preventDefault()}
        >
          {t("lub.upsell.start_trial", { ns: "pro" })}
        </button>
        <button type="button" onClick={(event) => event.preventDefault()}>
          {t("lub.upsell.compare_link", { ns: "pro" })}
        </button>
        <button type="button" onClick={onClose}>
          {t("lub.upsell.close", { ns: "pro" })}
        </button>
      </div>
    </div>
  );
}
