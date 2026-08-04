import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { Card } from "@/components/ui/card";
import type { UsageTrendBucketView } from "@/types/usageDashboard";
import { cn } from "@/lib/utils";
import { formatTokensCompact } from "./usagePresentation";

export interface ProviderActivityDay {
  key: string;
  date: Date;
  eventCount: number;
  totalTokens: number;
  level: 0 | 1 | 2 | 3 | 4;
}

interface ProviderActivityHeatmapProps {
  buckets: UsageTrendBucketView[];
  startAt: number;
  endAt: number;
  isLoading?: boolean;
}

const CELL_TONES = [
  "bg-muted/55",
  "bg-primary/20",
  "bg-primary/40",
  "bg-primary/65",
  "bg-primary",
] as const;

function localDateKey(date: Date): string {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

function startOfLocalDay(timestampMs: number): Date {
  const date = new Date(timestampMs);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

export function buildProviderActivityDays(
  buckets: UsageTrendBucketView[],
  startAt: number,
  endAt: number,
): ProviderActivityDay[] {
  if (
    !Number.isFinite(startAt) ||
    !Number.isFinite(endAt) ||
    startAt >= endAt
  ) {
    return [];
  }

  const aggregateByDay = new Map<
    string,
    { eventCount: number; totalTokens: number }
  >();
  for (const bucket of buckets) {
    const key = localDateKey(new Date(bucket.startAt * 1_000));
    const current = aggregateByDay.get(key) ?? {
      eventCount: 0,
      totalTokens: 0,
    };
    aggregateByDay.set(key, {
      eventCount: current.eventCount + bucket.eventCount,
      totalTokens: current.totalTokens + bucket.totalTokens,
    });
  }

  const days: ProviderActivityDay[] = [];
  const endMs = endAt * 1_000;
  for (
    let cursor = startOfLocalDay(startAt * 1_000);
    cursor.getTime() < endMs;
    cursor = new Date(
      cursor.getFullYear(),
      cursor.getMonth(),
      cursor.getDate() + 1,
    )
  ) {
    const key = localDateKey(cursor);
    const aggregate = aggregateByDay.get(key) ?? {
      eventCount: 0,
      totalTokens: 0,
    };
    days.push({
      key,
      date: cursor,
      eventCount: aggregate.eventCount,
      totalTokens: aggregate.totalTokens,
      level: 0,
    });
  }

  const maxTokens = days.reduce(
    (maximum, day) => Math.max(maximum, day.totalTokens),
    0,
  );
  return days.map((day) => {
    if (day.eventCount === 0 && day.totalTokens === 0) return day;
    const normalized = maxTokens ? Math.sqrt(day.totalTokens / maxTokens) : 0;
    return {
      ...day,
      level: Math.max(1, Math.ceil(normalized * 4)) as 1 | 2 | 3 | 4,
    };
  });
}

function recentActivityStreak(days: ProviderActivityDay[]): number {
  let index = days.length - 1;
  while (index >= 0 && !days[index]?.eventCount) index -= 1;
  let streak = 0;
  while (index >= 0 && days[index]?.eventCount) {
    streak += 1;
    index -= 1;
  }
  return streak;
}

function latestActiveActivityDay(
  days: ProviderActivityDay[],
): ProviderActivityDay | undefined {
  for (let index = days.length - 1; index >= 0; index -= 1) {
    if (days[index]?.eventCount) return days[index];
  }
  return undefined;
}

export function ProviderActivityHeatmap({
  buckets,
  startAt,
  endAt,
  isLoading = false,
}: ProviderActivityHeatmapProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const days = useMemo(
    () => buildProviderActivityDays(buckets, startAt, endAt),
    [buckets, endAt, startAt],
  );
  const [hoveredDayKey, setHoveredDayKey] = useState<string | null>(null);
  const [keyboardDayKey, setKeyboardDayKey] = useState<string | null>(null);
  const [focusedDayKey, setFocusedDayKey] = useState<string | null>(null);
  const [pinnedDayKey, setPinnedDayKey] = useState<string | null>(null);
  const scrollViewportRef = useRef<HTMLDivElement>(null);
  const didPositionInitialScrollRef = useRef(false);
  const dayByKey = useMemo(
    () => new Map(days.map((day) => [day.key, day])),
    [days],
  );
  const leadingBlanks = days[0]?.date.getDay() ?? 0;
  const weekCount = Math.ceil((leadingBlanks + days.length) / 7);
  const trailingBlanks = weekCount * 7 - leadingBlanks - days.length;
  const activeDays = days.filter((day) => day.eventCount > 0).length;
  const streak = recentActivityStreak(days);
  const latestActiveDay = latestActiveActivityDay(days);
  const fallbackDayKey = latestActiveDay?.key ?? days.at(-1)?.key ?? null;
  const effectiveKeyboardDayKey = dayByKey.has(keyboardDayKey ?? "")
    ? keyboardDayKey
    : fallbackDayKey;
  const hoveredDay = hoveredDayKey ? dayByKey.get(hoveredDayKey) : undefined;
  const focusedDay = focusedDayKey ? dayByKey.get(focusedDayKey) : undefined;
  const pinnedDay = pinnedDayKey ? dayByKey.get(pinnedDayKey) : undefined;
  const detailDay = hoveredDay ?? focusedDay ?? pinnedDay;
  const monthByWeek = new Map<number, string>();
  days.forEach((day, index) => {
    const week = Math.floor((leadingBlanks + index) / 7);
    if (index === 0 || day.date.getDate() === 1) {
      monthByWeek.set(
        week,
        day.date.toLocaleDateString(locale, { month: "short" }),
      );
    }
  });

  const gridStyle = {
    gridTemplateRows: "repeat(7, var(--activity-slot))",
    gridAutoColumns: "var(--activity-slot)",
    gridAutoFlow: "column",
  } as CSSProperties;
  const monthStyle = {
    gridTemplateColumns: `repeat(${Math.max(weekCount, 1)}, var(--activity-slot))`,
  } as CSSProperties;

  useLayoutEffect(() => {
    const viewport = scrollViewportRef.current;
    if (
      !viewport ||
      isLoading ||
      days.length === 0 ||
      didPositionInitialScrollRef.current
    ) {
      return;
    }

    viewport.scrollLeft = Math.max(
      0,
      viewport.scrollWidth - viewport.clientWidth,
    );
    didPositionInitialScrollRef.current = true;
  }, [days.length, isLoading]);

  const moveKeyboardFocus = (
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    const delta =
      event.key === "ArrowLeft"
        ? -7
        : event.key === "ArrowRight"
          ? 7
          : event.key === "ArrowUp"
            ? -1
            : event.key === "ArrowDown"
              ? 1
              : null;
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? days.length - 1
          : delta == null
            ? null
            : Math.min(days.length - 1, Math.max(0, index + delta));
    if (event.key === "Escape") {
      event.preventDefault();
      setPinnedDayKey(null);
      return;
    }
    if (nextIndex == null || nextIndex === index) return;
    event.preventDefault();
    const buttons = event.currentTarget
      .closest("[data-activity-grid]")
      ?.querySelectorAll<HTMLButtonElement>("button[data-activity-date]");
    buttons?.[nextIndex]?.focus();
  };

  return (
    <section aria-labelledby="provider-activity-heading">
      <Card className="relative overflow-hidden p-5">
        <div className="flex flex-wrap items-start justify-between gap-x-5 gap-y-2">
          <div>
            <h2
              id="provider-activity-heading"
              className="text-sm font-semibold"
            >
              {t("usageDashboard.dailyActivity", {
                defaultValue: "Daily activity",
              })}
            </h2>
            <p className="mt-1 text-[11px] text-muted-foreground">
              {t("usageDashboard.activityRange", {
                defaultValue: "Last 12 months · local calendar days",
              })}
            </p>
          </div>
          <div className="ml-auto min-w-[190px] text-right">
            <span className="text-xs text-muted-foreground metric">
              {t("usageDashboard.activeDays", {
                count: activeDays,
                defaultValue: "{{count}} active days",
              })}
            </span>
            <div
              className="mt-1 min-h-8 text-[11px]"
              data-activity-detail
              aria-live="polite"
            >
              {detailDay ? (
                <>
                  <strong className="block font-medium text-foreground">
                    {detailDay.date.toLocaleDateString(locale, {
                      year: "numeric",
                      month: "short",
                      day: "numeric",
                    })}
                  </strong>
                  <span className="mt-0.5 block text-muted-foreground metric">
                    {formatTokensCompact(detailDay.totalTokens)} Token ·{" "}
                    {t("usageDashboard.recordCount", {
                      count: detailDay.eventCount,
                      defaultValue: "{{count}} records",
                    })}
                  </span>
                </>
              ) : (
                <span className="text-muted-foreground/75">
                  {t("usageDashboard.activityInteractionHint", {
                    defaultValue: "Hover to preview · Click to pin",
                  })}
                </span>
              )}
            </div>
          </div>
        </div>

        {isLoading ? (
          <div
            className="mt-6 h-32 animate-pulse rounded-lg bg-muted/45"
            aria-hidden="true"
          />
        ) : (
          <div
            ref={scrollViewportRef}
            data-activity-scroll
            // The year grid is 53 columns wide. Sizing the slot in `cqw` lets it
            // grow to fill the card instead of stranding empty space beside a
            // fixed-width grid; the clamp keeps cells legible and still scrolls
            // horizontally once the card is too narrow for the minimum size.
            className="mt-5 overflow-x-auto overscroll-x-contain pb-1 [container-type:inline-size] [--activity-slot:clamp(12px,1.82cqw,20px)] [--activity-cell:calc(var(--activity-slot)-4px)]"
            aria-label={t("usageDashboard.activityChartLabel", {
              defaultValue: "Daily token activity for the last 12 months",
            })}
          >
            <div className="relative w-max min-w-full">
              <div className="grid w-max" style={gridStyle} data-activity-grid>
                {Array.from({ length: leadingBlanks }, (_, index) => (
                  <span key={`leading-${index}`} aria-hidden="true" />
                ))}
                {days.map((day, index) => {
                  const detail = `${day.date.toLocaleDateString(locale, {
                    year: "numeric",
                    month: "short",
                    day: "numeric",
                  })}: ${day.totalTokens.toLocaleString(locale)} Token · ${t(
                    "usageDashboard.recordCount",
                    {
                      count: day.eventCount,
                      defaultValue: "{{count}} records",
                    },
                  )}`;
                  return (
                    <button
                      key={day.key}
                      type="button"
                      tabIndex={day.key === effectiveKeyboardDayKey ? 0 : -1}
                      data-activity-date={day.key}
                      data-activity-level={day.level}
                      aria-label={detail}
                      aria-pressed={day.key === pinnedDayKey}
                      className="group flex h-[var(--activity-slot)] w-[var(--activity-slot)] cursor-pointer items-center justify-center rounded-[4px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                      onMouseEnter={() => setHoveredDayKey(day.key)}
                      onMouseLeave={() => setHoveredDayKey(null)}
                      onClick={() =>
                        setPinnedDayKey((currentDayKey) =>
                          currentDayKey === day.key ? null : day.key,
                        )
                      }
                      onFocus={() => {
                        setKeyboardDayKey(day.key);
                        setFocusedDayKey(day.key);
                      }}
                      onBlur={() =>
                        setFocusedDayKey((currentDayKey) =>
                          currentDayKey === day.key ? null : currentDayKey,
                        )
                      }
                      onKeyDown={(event) => moveKeyboardFocus(event, index)}
                    >
                      <span
                        data-activity-cell-visual
                        className={cn(
                          "pointer-events-none h-[var(--activity-cell)] w-[var(--activity-cell)] rounded-[3px] transition-transform duration-100 ease-out group-hover:scale-110 group-focus-visible:scale-110",
                          CELL_TONES[day.level],
                          day.key === pinnedDayKey &&
                            "ring-2 ring-primary ring-offset-1 ring-offset-card",
                        )}
                      />
                    </button>
                  );
                })}
                {Array.from({ length: trailingBlanks }, (_, index) => (
                  <span key={`trailing-${index}`} aria-hidden="true" />
                ))}
              </div>

              <div className="mt-2 grid w-max" style={monthStyle}>
                {Array.from({ length: weekCount }, (_, week) => (
                  <span
                    key={week}
                    className="overflow-visible whitespace-nowrap text-[10px] text-muted-foreground/70"
                  >
                    {monthByWeek.get(week) ?? ""}
                  </span>
                ))}
              </div>
            </div>

            <div className="mt-3 flex min-w-max items-center justify-between gap-6 text-[10px] text-muted-foreground">
              <span>
                {t("usageDashboard.recentStreak", {
                  count: streak,
                  defaultValue: "Recent streak {{count}} days",
                })}
              </span>
              <div className="flex items-center gap-1.5" aria-hidden="true">
                <span>
                  {t("usageDashboard.lowActivity", { defaultValue: "Low" })}
                </span>
                {CELL_TONES.map((tone, index) => (
                  <span
                    key={tone}
                    className={cn("h-2.5 w-2.5 rounded-[2px]", tone)}
                    data-legend-level={index}
                  />
                ))}
                <span>
                  {t("usageDashboard.highActivity", { defaultValue: "High" })}
                </span>
              </div>
            </div>
          </div>
        )}
      </Card>
    </section>
  );
}
