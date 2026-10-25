import { useTranslation } from "react-i18next";
import { pricing } from "@/lib/pricing";
import {
  compareGroups,
  type CompareCell,
  type CompareRow,
} from "./proCompareModel";

function Mark({ cell }: { cell: CompareCell }) {
  const { t } = useTranslation();
  if (cell.kind === "check") {
    return (
      <svg
        className="lub-check"
        viewBox="0 0 16 16"
        role="img"
        aria-label={t("lub.procompare.included")}
      >
        <path d="M3 8.5 6.2 12 13 4" />
      </svg>
    );
  }
  if (cell.kind === "text") {
    return <span>{t(`lub.procompare.${cell.key}`)}</span>;
  }
  return (
    <span className="lub-muted" aria-label={t("lub.procompare.not_included")}>
      —
    </span>
  );
}

function Row({ row }: { row: CompareRow }) {
  const { t } = useTranslation();
  return (
    <tr>
      <th scope="row">
        {t(`lub.procompare.${row.labelKey}`)}
        {row.beta ? (
          <span className="lub-tag">{t("lub.procompare.beta")}</span>
        ) : null}
      </th>
      <td>
        <Mark cell={row.free} />
      </td>
      <td>
        <Mark cell={row.pro} />
      </td>
    </tr>
  );
}

export function ProCompareTable() {
  const { t } = useTranslation();
  const price =
    pricing.phase === "published" && pricing.monthlyUsd
      ? pricing.monthlyUsd
      : t("lub.procompare.price_later");

  return (
    <div className="lub-compare" data-testid="pro-compare">
      <h1 className="lub-title">{t("lub.procompare.title")}</h1>
      <p className="lub-muted">{t("lub.procompare.subtitle")}</p>
      <div className="lub-price-row">
        <div className="lub-card lub-price-card">
          <b>Free</b>
          <span>{t("lub.procompare.free_forever")}</span>
        </div>
        <div className="lub-card lub-price-card">
          <b>Pro</b>
          <span>{price}</span>
        </div>
      </div>
      <table>
        <thead>
          <tr>
            <th scope="col" />
            <th scope="col">Free</th>
            <th scope="col">Pro</th>
          </tr>
        </thead>
        <tbody>
          {compareGroups.map((group) => (
            <FragmentRows key={group.id} group={group} />
          ))}
        </tbody>
      </table>
      <p className="lub-muted">{t("lub.procompare.offline_note")}</p>
      <button
        type="button"
        className="lub-btn lub-btn-primary"
        onClick={(event) => event.preventDefault()}
      >
        {t("lub.procompare.start_trial")}
      </button>
    </div>
  );
}

function FragmentRows({ group }: { group: (typeof compareGroups)[number] }) {
  const { t } = useTranslation();
  return (
    <>
      <tr className="lub-compare-group">
        <th scope="colgroup" colSpan={3}>
          {t(`lub.procompare.${group.labelKey}`)}
        </th>
      </tr>
      {group.rows.map((row) => (
        <Row key={row.id} row={row} />
      ))}
    </>
  );
}
