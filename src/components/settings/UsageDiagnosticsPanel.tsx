import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ShieldCheck } from "lucide-react";
import { useUnassignedUsageDiagnostics } from "@/lib/query/usageDashboard";
import { SettingsSection } from "./SettingsSection";

function formatTime(value: number) {
  return new Date(value * 1000).toLocaleString();
}

interface DiagnosticsSectionProps {
  title: string;
  count: number;
  emptyText: string;
  children: ReactNode;
}

function DiagnosticsSection({
  title,
  count,
  emptyText,
  children,
}: DiagnosticsSectionProps) {
  return (
    <section className="overflow-hidden rounded-xl border bg-card shadow-card">
      <header className="flex items-center justify-between gap-3 border-b border-border/60 bg-muted/30 px-4 py-2.5">
        <h3 className="text-sm font-medium">{title}</h3>
        <span className="rounded-full border border-border/60 bg-background px-2 py-0.5 text-xs tabular-nums text-muted-foreground">
          {count}
        </span>
      </header>
      {count === 0 ? (
        // A clear result deserves one quiet line, not a panel-sized empty state.
        <p className="px-4 py-2.5 text-sm text-muted-foreground">{emptyText}</p>
      ) : (
        <div className="divide-y divide-border/60">{children}</div>
      )}
    </section>
  );
}

function DiagnosticsRow({ title, meta }: { title: string; meta: ReactNode }) {
  return (
    <div className="px-4 py-3 text-sm">
      <div className="font-medium">{title}</div>
      <div className="mt-0.5 text-muted-foreground">{meta}</div>
    </div>
  );
}

export function UsageDiagnosticsPanel() {
  const { t } = useTranslation();
  const diagnostics = useUnassignedUsageDiagnostics();
  const data = diagnostics.data;

  if (diagnostics.isLoading) {
    return (
      <div className="py-12 text-center text-sm text-muted-foreground">
        {t("common.loading", { defaultValue: "Loading" })}
      </div>
    );
  }
  if (diagnostics.error) {
    return (
      <div role="alert" className="text-sm text-destructive">
        {diagnostics.error instanceof Error
          ? diagnostics.error.message
          : String(diagnostics.error)}
      </div>
    );
  }
  if (!data) return null;

  const timeRange = (first: number, last: number) =>
    `${formatTime(first)} – ${formatTime(last)}`;

  const unassignedCountText = t("usageDiagnostics.unassignedCount", {
    count: data.unassignedEventCount,
    defaultValue: `${data.unassignedEventCount} unassigned events`,
  });

  return (
    <div className="space-y-5 pb-6">
      <SettingsSection
        icon={ShieldCheck}
        title={t("usageDiagnostics.title", {
          defaultValue: "Usage ownership diagnostics",
        })}
        description={t("usageDiagnostics.description", {
          defaultValue:
            "Read-only aggregate history. The app never guesses or reassigns ownership.",
        })}
      >
        <div className="flex items-center gap-2.5 rounded-lg bg-muted/50 px-3.5 py-2.5 text-sm dark:bg-muted/30">
          <span
            aria-hidden="true"
            className={
              data.unassignedEventCount === 0
                ? "h-2 w-2 shrink-0 rounded-full bg-success"
                : "h-2 w-2 shrink-0 rounded-full bg-warning"
            }
          />
          <span className="font-medium tabular-nums">
            {unassignedCountText}
          </span>
        </div>
      </SettingsSection>

      <DiagnosticsSection
        title={t("usageDiagnostics.unassignedGroups", {
          defaultValue: "Unassigned groups",
        })}
        count={data.unassignedGroups.length}
        emptyText={t("usageDiagnostics.emptyUnassignedGroups", {
          defaultValue: "No unassigned groups",
        })}
      >
        {data.unassignedGroups.map((group) => (
          <DiagnosticsRow
            key={`${group.providerId}-${group.source}`}
            title={group.providerId}
            meta={
              <>
                {group.source} · {group.eventCount} ·{" "}
                {timeRange(group.firstOccurredAt, group.lastOccurredAt)}
              </>
            }
          />
        ))}
      </DiagnosticsSection>

      <DiagnosticsSection
        title={t("usageDiagnostics.archivedHistory", {
          defaultValue: "Archived Agent history",
        })}
        count={data.archivedAgentHistory.length}
        emptyText={t("usageDiagnostics.emptyArchivedHistory", {
          defaultValue: "No archived Agent history",
        })}
      >
        {data.archivedAgentHistory.map((summary) => (
          <DiagnosticsRow
            key={summary.agentModuleId}
            title={summary.agentModuleId}
            meta={
              <>
                {summary.eventCount} ·{" "}
                {timeRange(summary.firstOccurredAt, summary.lastOccurredAt)}
              </>
            }
          />
        ))}
      </DiagnosticsSection>

      <DiagnosticsSection
        title={t("usageDiagnostics.invalidLinks", {
          defaultValue: "Invalid link summaries",
        })}
        count={data.invalidLinkSummaries.length}
        emptyText={t("usageDiagnostics.emptyInvalidLinks", {
          defaultValue: "No invalid links",
        })}
      >
        {data.invalidLinkSummaries.map((summary) => (
          <DiagnosticsRow
            key={summary.reason}
            title={summary.reason}
            meta={
              <>
                <span>
                  {t("usageDiagnostics.invalidLinkCount", {
                    count: summary.linkCount,
                    defaultValue: `${summary.linkCount} invalid links`,
                  })}
                </span>{" "}
                · {timeRange(summary.firstCreatedAt, summary.lastCreatedAt)}
              </>
            }
          />
        ))}
      </DiagnosticsSection>
    </div>
  );
}
