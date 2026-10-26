import { useTranslation } from "react-i18next";
import {
  compareGroups,
  type CompareCell,
  type CompareGroup,
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
      <th scope="row">{t(`lub.procompare.${row.labelKey}`)}</th>
      <td>
        <Mark cell={row.free} />
      </td>
      <td>
        <Mark cell={row.pro} />
      </td>
    </tr>
  );
}

function GroupRows({ group }: { group: CompareGroup }) {
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

/** Free / Pro columns. Product names stay Free and Pro in every language. */
export function ProCompareTable() {
  const { t } = useTranslation();
  return (
    <div className="lub-compare" data-testid="pro-compare">
      <table aria-label={t("lub.upgrade.aria_table")}>
        <thead>
          <tr>
            <th scope="col" />
            <th scope="col">Free</th>
            <th scope="col">Pro</th>
          </tr>
        </thead>
        <tbody>
          {compareGroups.map((group) => (
            <GroupRows key={group.id} group={group} />
          ))}
        </tbody>
      </table>
    </div>
  );
}
