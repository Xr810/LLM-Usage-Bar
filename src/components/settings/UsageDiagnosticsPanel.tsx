import { useTranslation } from "react-i18next";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useUnassignedUsageDiagnostics } from "@/lib/query/usageDashboard";

function formatTime(value: number) {
  return new Date(value * 1000).toLocaleString();
}

export function UsageDiagnosticsPanel() {
  const { t } = useTranslation();
  const diagnostics = useUnassignedUsageDiagnostics();
  const data = diagnostics.data;

  if (diagnostics.isLoading) {
    return <div>{t("common.loading", { defaultValue: "Loading" })}</div>;
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

  return (
    <div className="space-y-4 pb-6">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {t("usageDiagnostics.title", {
              defaultValue: "Usage ownership diagnostics",
            })}
          </CardTitle>
          <CardDescription>
            {t("usageDiagnostics.description", {
              defaultValue:
                "Read-only aggregate history. The app never guesses or reassigns ownership.",
            })}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {t("usageDiagnostics.unassignedCount", {
            count: data.unassignedEventCount,
            defaultValue: `${data.unassignedEventCount} unassigned events`,
          })}
        </CardContent>
      </Card>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold">
          {t("usageDiagnostics.unassignedGroups", {
            defaultValue: "Unassigned groups",
          })}
        </h3>
        {data.unassignedGroups.map((group) => (
          <div
            key={`${group.providerId}-${group.source}`}
            className="rounded-lg border p-3 text-sm"
          >
            <div className="font-medium">{group.providerId}</div>
            <div className="text-muted-foreground">
              {group.source} · {group.eventCount} ·{" "}
              {timeRange(group.firstOccurredAt, group.lastOccurredAt)}
            </div>
          </div>
        ))}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold">
          {t("usageDiagnostics.archivedHistory", {
            defaultValue: "Archived Agent history",
          })}
        </h3>
        {data.archivedAgentHistory.map((summary) => (
          <div
            key={summary.agentModuleId}
            className="rounded-lg border p-3 text-sm"
          >
            <div className="font-medium">{summary.agentModuleId}</div>
            <div className="text-muted-foreground">
              {summary.eventCount} ·{" "}
              {timeRange(summary.firstOccurredAt, summary.lastOccurredAt)}
            </div>
          </div>
        ))}
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-semibold">
          {t("usageDiagnostics.invalidLinks", {
            defaultValue: "Invalid link summaries",
          })}
        </h3>
        {data.invalidLinkSummaries.map((summary) => (
          <div key={summary.reason} className="rounded-lg border p-3 text-sm">
            <div className="font-medium">{summary.reason}</div>
            <div className="text-muted-foreground">
              <span>
                {t("usageDiagnostics.invalidLinkCount", {
                  count: summary.linkCount,
                  defaultValue: `${summary.linkCount} invalid links`,
                })}
              </span>{" "}
              · {timeRange(summary.firstCreatedAt, summary.lastCreatedAt)}
            </div>
          </div>
        ))}
      </section>
    </div>
  );
}
