import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { GuardChangedResult } from "@/lib/api/guard";
import type { AllowanceUsd, ConvertedBudget, UpgradeNotice } from "@/types";
import type { LicenseStatus } from "@/types/license";
import { debugUpgradeNotice } from "@/lib/upgradeNoticeDebug";
import {
  convertedBudgetName,
  joinBudgetNames,
  partitionConvertedBudgets,
  upgradeNoticeVisible,
  usdLabel,
} from "./upgradeNoticeModel";

const SPENT_MARK = "\uE000";

export interface UpgradeHardStopNoticeProps {
  notice: UpgradeNotice | null;
  license: LicenseStatus | null;
  allowanceUsd?: AllowanceUsd;
  onAck: (noticeId: string) => Promise<void> | void;
  onSetHardStop: (budgetId: string) => Promise<GuardChangedResult>;
  onAllowMoreToday: () => Promise<void> | void;
  onManageBudgets: () => void;
}

function LockIcon() {
  return (
    <svg className="lub-icon" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="5" y="10.5" width="14" height="10" rx="2" />
      <path d="M8.5 10.5V7.5a3.5 3.5 0 0 1 7 0v3" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      className="lub-icon lub-icon-14"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg
      className="lub-icon lub-icon-14"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

function ErrorIcon() {
  return (
    <svg
      className="lub-icon lub-icon-14"
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5v5.5" />
      <path d="M12 16.3v.2" />
    </svg>
  );
}

function Amount({ budget, over }: { budget: ConvertedBudget; over: boolean }) {
  const { t } = useTranslation();
  const spent = usdLabel(budget.spentTodayUsd);
  const cap = usdLabel(budget.dailyCapUsd);
  const [prefix, suffix = ""] = t("guard.upgradeNotice.amountPerDay", {
    spent: SPENT_MARK,
    cap,
  }).split(SPENT_MARK);
  return (
    <span className={`lub-upgrade-amt${over ? " is-over" : ""}`}>
      {prefix}
      <span className="lub-upgrade-spent">{spent}</span>
      <span className="lub-upgrade-of">{suffix}</span>
    </span>
  );
}

export function UpgradeHardStopNotice({
  notice,
  license,
  allowanceUsd = "5",
  onAck,
  onSetHardStop,
  onAllowMoreToday,
  onManageBudgets,
}: UpgradeHardStopNoticeProps) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(true);
  const [switched, setSwitched] = useState<ReadonlySet<string>>(new Set());
  const [failedIds, setFailedIds] = useState<string[]>([]);
  const [trackedId, setTrackedId] = useState(notice?.noticeId);
  const [switchAllBusy, setSwitchAllBusy] = useState(false);
  const [allowBusy, setAllowBusy] = useState(false);
  const [acking, setAcking] = useState(false);
  const switchAllLock = useRef(false);
  const allowLock = useRef(false);
  const ackLock = useRef(false);

  if (notice?.noticeId !== trackedId) {
    setTrackedId(notice?.noticeId);
    setSwitched(new Set());
    setFailedIds([]);
    setOpen(true);
  }

  if (!upgradeNoticeVisible(notice, license)) return null;

  const language = i18n.resolvedLanguage || i18n.language || "en";
  const sharedName = t("guard.upgradeNotice.sharedName");
  const nameOf = (budget: ConvertedBudget) =>
    convertedBudgetName(budget, sharedName);
  const { over, rest } = partitionConvertedBudgets(notice.budgets, switched);
  const amount = usdLabel(allowanceUsd);
  const visibleFailed = failedIds.filter((id) => !switched.has(id));
  const failedNames = notice.budgets
    .filter((budget) => visibleFailed.includes(budget.budgetId))
    .map(nameOf);
  const remaining = notice.budgets.filter(
    (budget) => !switched.has(budget.budgetId),
  );

  const markSwitched = (budgetId: string) => {
    setSwitched((current) => {
      const next = new Set(current);
      next.add(budgetId);
      return next;
    });
    setFailedIds((current) => current.filter((id) => id !== budgetId));
  };

  const switchOne = async (budgetId: string) => {
    try {
      const result = await onSetHardStop(budgetId);
      if (result.changed) markSwitched(budgetId);
    } catch (error) {
      debugUpgradeNotice(error);
    }
  };

  const switchAll = async () => {
    if (switchAllLock.current) return;
    const targets =
      visibleFailed.length > 0
        ? notice.budgets.filter((budget) =>
            visibleFailed.includes(budget.budgetId),
          )
        : remaining;
    if (targets.length === 0) return;
    switchAllLock.current = true;
    setSwitchAllBusy(true);
    const failed: string[] = [];
    try {
      for (const budget of targets) {
        try {
          const result = await onSetHardStop(budget.budgetId);
          if (result.changed) {
            markSwitched(budget.budgetId);
          } else {
            failed.push(budget.budgetId);
          }
        } catch (error) {
          debugUpgradeNotice(error);
          failed.push(budget.budgetId);
        }
      }
      setFailedIds(failed);
    } finally {
      switchAllLock.current = false;
      setSwitchAllBusy(false);
    }
  };

  const allow = async () => {
    if (allowLock.current) return;
    allowLock.current = true;
    setAllowBusy(true);
    try {
      await onAllowMoreToday();
    } catch (error) {
      debugUpgradeNotice(error);
    } finally {
      allowLock.current = false;
      setAllowBusy(false);
    }
  };

  const dismiss = async () => {
    if (ackLock.current) return;
    ackLock.current = true;
    setAcking(true);
    try {
      await onAck(notice.noticeId);
    } finally {
      ackLock.current = false;
      setAcking(false);
    }
  };

  return (
    <section
      className="lub-upgrade-notice"
      data-testid="upgrade-hard-stop-notice"
      role="region"
      aria-labelledby="upgrade-hard-stop-title"
    >
      <div className="lub-upgrade-head">
        <span className="lub-upgrade-icon" aria-hidden="true">
          <LockIcon />
        </span>
        <div className="lub-upgrade-copy">
          <b id="upgrade-hard-stop-title">{t("guard.upgradeNotice.title")}</b>
          <p className="lub-upgrade-body">
            {t("guard.upgradeNotice.body", { count: notice.budgets.length })}
          </p>
          <p className="lub-upgrade-monthly">
            {t("guard.upgradeNotice.monthlyNote")}
          </p>
        </div>
        <button
          type="button"
          className="lub-upgrade-close"
          aria-label={t("guard.upgradeNotice.dismissAria")}
          onClick={() => void dismiss()}
          disabled={acking}
        >
          <CloseIcon />
        </button>
      </div>

      {over.length > 0 ? (
        <div className="lub-upgrade-section">
          <div className="lub-upgrade-section-title is-warn">
            <i className="lub-upgrade-dot" aria-hidden="true" />
            <span>
              {t("guard.upgradeNotice.overTodayHeading", { n: over.length })}
            </span>
          </div>
          {over.map((budget) => (
            <div className="lub-upgrade-over" key={budget.budgetId}>
              <div className="lub-upgrade-line">
                <span className="lub-upgrade-name">{nameOf(budget)}</span>
                <Amount budget={budget} over />
              </div>
              <div className="lub-upgrade-actions">
                <button
                  type="button"
                  className="lub-btn lub-btn-primary"
                  onClick={() => void allow()}
                  disabled={allowBusy}
                >
                  {t("guard.upgradeNotice.allowMoreToday", { amount })}
                </button>
                <button
                  type="button"
                  className="lub-btn"
                  onClick={() => void switchOne(budget.budgetId)}
                  disabled={switchAllBusy}
                >
                  {t("guard.upgradeNotice.switchBack")}
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : null}

      <div className="lub-upgrade-section">
        <button
          type="button"
          className="lub-upgrade-disclosure"
          aria-expanded={open}
          aria-label={t("guard.upgradeNotice.toggleListAria")}
          onClick={() => setOpen((current) => !current)}
        >
          <span className="lub-upgrade-section-title">
            {over.length > 0
              ? t("guard.upgradeNotice.otherHeading", { m: rest.length })
              : t("guard.upgradeNotice.allHeading", { m: rest.length })}
          </span>
          <ChevronIcon />
        </button>
        <div className="lub-upgrade-list" hidden={!open}>
          {rest.map((budget) => {
            const done = switched.has(budget.budgetId);
            return (
              <div className="lub-upgrade-row" key={budget.budgetId}>
                <span className="lub-upgrade-name">{nameOf(budget)}</span>
                <span className="lub-upgrade-row-side">
                  <Amount budget={budget} over={false} />
                  {done ? (
                    <span className="lub-upgrade-tag">
                      {t("guard.upgradeNotice.switchedBack")}
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="lub-link lub-upgrade-switch"
                      onClick={() => void switchOne(budget.budgetId)}
                      disabled={switchAllBusy}
                    >
                      {t("guard.upgradeNotice.switchBack")}
                    </button>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      </div>

      <div className="lub-upgrade-foot">
        <div className="lub-upgrade-foot-actions">
          <button
            type="button"
            className="lub-btn"
            onClick={() => void switchAll()}
            disabled={switchAllBusy || remaining.length === 0}
          >
            {t("guard.upgradeNotice.switchAllBack")}
          </button>
          <button
            type="button"
            className="lub-btn lub-btn-primary"
            onClick={() => void dismiss()}
            disabled={acking}
          >
            {t("guard.upgradeNotice.gotIt")}
          </button>
        </div>
        {visibleFailed.length > 0 ? (
          <p className="lub-upgrade-error" role="alert">
            <ErrorIcon />
            <span>
              {t("guard.upgradeNotice.switchAllPartialError", {
                n: visibleFailed.length,
                names: joinBudgetNames(failedNames, language),
              })}
            </span>
          </p>
        ) : null}
        <button
          type="button"
          className="lub-link lub-upgrade-manage"
          onClick={onManageBudgets}
        >
          {t("guard.upgradeNotice.manageBudgets")}
        </button>
      </div>
    </section>
  );
}
