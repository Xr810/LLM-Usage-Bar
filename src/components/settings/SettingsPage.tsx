import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { UsageDashboardPage } from "@/components/usage-dashboard/UsageDashboardPage";
import { useTranslation } from "react-i18next";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportSuccess?: () => void | Promise<void>;
  defaultTab?: string;
}

/**
 * Milestone-3 settings surface.
 *
 * Legacy callers may still pass historical tab names. They intentionally all
 * resolve to the provider-aware usage surface instead of mounting a hidden or
 * blank legacy panel. The legacy component modules remain in the repository
 * for the compatibility window but are not reachable from this render tree.
 */
export function SettingsPage({ open, onOpenChange }: SettingsDialogProps) {
  const { t } = useTranslation();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[90vh] max-w-6xl flex-col overflow-hidden">
        <DialogHeader>
          <DialogTitle>
            {t("usageDashboard.title", { defaultValue: "Usage dashboard" })}
          </DialogTitle>
          <DialogDescription>
            {t("usageDashboard.description", {
              defaultValue:
                "Provider-aware tokens, costs and subscription quota",
            })}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto px-6">
          <UsageDashboardPage />
        </div>
      </DialogContent>
    </Dialog>
  );
}
