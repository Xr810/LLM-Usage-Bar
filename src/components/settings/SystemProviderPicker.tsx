import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ProviderIcon } from "@/components/ProviderIcon";
import { dashboardProviderIcon } from "@/components/usage-dashboard/usagePresentation";
import type { UsageProviderView } from "@/types/usageDashboard";

interface SystemProviderPickerProps {
  /** Every built-in Provider, in catalogue order — picked or not. */
  providers: UsageProviderView[];
  onToggle: (provider: UsageProviderView, picked: boolean) => void;
  isPending?: boolean;
}

/**
 * The catalogue, behind a control.
 *
 * Every built-in Provider used to be rendered as a full card, so a person
 * monitoring two accounts scrolled past eighteen they had never touched. The
 * catalogue is a thing you shop from once; the list below is what you own.
 */
export function SystemProviderPicker({
  providers,
  onToggle,
  isPending = false,
}: SystemProviderPickerProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");

  const query = search.trim().toLocaleLowerCase();
  const matches = useMemo(
    () =>
      providers.filter(
        (provider) =>
          !query ||
          provider.name.toLocaleLowerCase().includes(query) ||
          (provider.canonicalEndpoint ?? "")
            .toLocaleLowerCase()
            .includes(query),
      ),
    [providers, query],
  );
  const pickedCount = providers.filter((provider) => provider.enabled).length;

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setSearch("");
      }}
    >
      <PopoverTrigger asChild>
        <Button variant="outline" className="w-full border-dashed">
          <Plus className="mr-1.5 size-4" aria-hidden="true" />
          {t("usageDashboard.addBuiltInProvider", {
            defaultValue: "Add a built-in Provider",
          })}
          <span className="ml-1.5 text-xs text-muted-foreground metric">
            {pickedCount}/{providers.length}
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[min(24rem,90vw)] overflow-hidden p-0"
      >
        <div className="relative border-b p-2">
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute left-4 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            autoFocus
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label={t("usageDashboard.searchProviders", {
              defaultValue: "Search Providers",
            })}
            placeholder={t("usageDashboard.searchProvidersPlaceholder", {
              defaultValue: "Search by Provider name or endpoint...",
            })}
            className="pl-9"
          />
        </div>
        {/* Native overflow, not the styled ScrollArea: that one sizes its
            viewport with `h-full`, which has no definite height to resolve
            against under a `max-h`, so the list was clipped and never
            scrolled. A max-height still lets a filtered list shrink. */}
        <div className="max-h-72 overflow-y-auto overscroll-contain">
          <ul className="p-1">
            {matches.map((provider) => {
              const { icon, iconColor } = dashboardProviderIcon(provider);
              return (
                <li key={provider.id}>
                  {/* A whole-row label: the hit area for a checkbox this small
                      should be the line it names, not the box. */}
                  <label className="flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 hover:bg-muted/60">
                    <Checkbox
                      checked={provider.enabled}
                      disabled={isPending}
                      onCheckedChange={(checked) =>
                        onToggle(provider, checked === true)
                      }
                      aria-label={provider.name}
                    />
                    <ProviderIcon
                      icon={icon}
                      color={iconColor}
                      name={provider.name}
                      size={20}
                      className="shrink-0 rounded-[5px]"
                    />
                    <span className="min-w-0 flex-1 truncate text-sm">
                      {provider.name}
                    </span>
                  </label>
                </li>
              );
            })}
            {matches.length === 0 ? (
              <li className="px-2 py-6 text-center text-sm text-muted-foreground">
                {t("usageDashboard.noMatchingProviders", {
                  defaultValue: "No Providers match this search.",
                })}
              </li>
            ) : null}
          </ul>
        </div>
      </PopoverContent>
    </Popover>
  );
}
