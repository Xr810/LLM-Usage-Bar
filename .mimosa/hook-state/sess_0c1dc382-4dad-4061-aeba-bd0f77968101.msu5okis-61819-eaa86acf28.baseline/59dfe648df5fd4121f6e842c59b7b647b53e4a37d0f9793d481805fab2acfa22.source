import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { QuotaWindowPaceView } from "@/types/usageDashboard";
import { QuotaPaceDetails } from "./QuotaPaceDetails";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: Record<string, unknown>) => {
      const fallback = (options?.defaultValue as string) ?? _key;
      return fallback.replace(/\{\{(\w+)\}\}/g, (match, token: string) =>
        options && token in options ? String(options[token]) : match,
      );
    },
    i18n: { language: "en", resolvedLanguage: "en" },
  }),
}));

const RESETS_AT = "2026-07-14T01:00:00.000Z";

function pace(overrides: Partial<QuotaWindowPaceView> = {}) {
  return {
    status: "green",
    paceBasis: "measured",
    burnRatePercentPerHour: "12",
    ...overrides,
  } as QuotaWindowPaceView;
}

function open() {
  fireEvent.click(screen.getByRole("button", { name: /Why this colour/ }));
}

describe("QuotaPaceDetails", () => {
  it("stays collapsed until asked", () => {
    render(<QuotaPaceDetails pace={pace()} resetsAt={RESETS_AT} />);

    expect(screen.queryByText("Burn rate")).toBeNull();
    open();
    expect(screen.getByText("Burn rate")).toBeInTheDocument();
    expect(screen.getByText("12%/h")).toBeInTheDocument();
  });

  it("renders nothing when there was no clock to project against", () => {
    const { container } = render(
      <QuotaPaceDetails pace={pace({ paceBasis: "static" })} resetsAt={null} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("leads with whether the quota outlasts the reset", () => {
    render(
      <QuotaPaceDetails
        pace={pace({ projectedExhaustAt: "2026-07-14T03:00:00.000Z" })}
        resetsAt={RESETS_AT}
      />,
    );
    open();

    expect(screen.getByText("After the reset")).toBeInTheDocument();
  });

  it("shows a clock time when the quota runs out first", () => {
    render(
      <QuotaPaceDetails
        pace={pace({ projectedExhaustAt: "2026-07-14T00:30:00.000Z" })}
        resetsAt={RESETS_AT}
      />,
    );
    open();

    expect(screen.queryByText("After the reset")).toBeNull();
    expect(screen.getByText("Runs out")).toBeInTheDocument();
  });

  it("explains an idle window", () => {
    render(
      <QuotaPaceDetails
        pace={pace({ paceBasis: "idle", burnRatePercentPerHour: "0" })}
        resetsAt={RESETS_AT}
      />,
    );
    open();

    expect(screen.getByText("Idle")).toBeInTheDocument();
    expect(screen.getByText("Not at this rate")).toBeInTheDocument();
  });

  it("quantifies the rhythm in both directions and hides it at parity", () => {
    const { unmount } = render(
      <QuotaPaceDetails
        pace={pace({ rhythmAdjustment: "0.3" })}
        resetsAt={RESETS_AT}
      />,
    );
    open();
    expect(screen.getByText("Usually 70% quieter now")).toBeInTheDocument();
    unmount();

    render(
      <QuotaPaceDetails
        pace={pace({ rhythmAdjustment: "1.5" })}
        resetsAt={RESETS_AT}
      />,
    );
    open();
    expect(screen.getByText("Usually 50% busier now")).toBeInTheDocument();
  });

  it("names the fallback basis so a thin estimate is not read as measured", () => {
    render(
      <QuotaPaceDetails
        pace={pace({ paceBasis: "window_average" })}
        resetsAt={RESETS_AT}
      />,
    );
    open();

    expect(
      screen.getByText("This window's average — not enough recent samples"),
    ).toBeInTheDocument();
  });
});
