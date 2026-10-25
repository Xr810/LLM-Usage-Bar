import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { CAP_LINE_RATIO } from "./capMath";
import {
  betaPanelFixture,
  designPanelFixture,
  killedPanelFixture,
} from "./panelFixture";
import { TrayPanelV5 } from "./TrayPanelV5";

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  await i18n.changeLanguage("zh");
});

function renderPanel(model = designPanelFixture()) {
  return render(
    <TrayPanelV5
      model={model}
      locale="zh-CN"
      onOpenTrends={() => undefined}
      onOpenSettings={() => undefined}
    />,
  );
}

describe("TrayPanelV5 design fixture", () => {
  it("draws only the total-card lines, both in the shared cap style", () => {
    renderPanel();
    const markers = document.querySelectorAll("[data-lub-marker]");
    expect(markers).toHaveLength(2);
    expect(markers[0]).toHaveAttribute("data-lub-marker", "day");
    expect(markers[1]).toHaveAttribute("data-lub-marker", "month");
    for (const marker of markers) {
      expect(marker.getAttribute("style") ?? "").toContain(
        `${(CAP_LINE_RATIO * 100).toFixed(1)}`,
      );
      expect(marker.className).not.toMatch(/warn|beta|orange/);
    }
    expect(screen.queryByText("β")).toBeNull();
    expect(screen.queryByText("受上限管控")).toBeNull();
    expect(screen.queryByText("仅监控")).toBeNull();
    expect(screen.queryByText("可硬停")).toBeNull();
    expect(screen.queryByText("显示文字标签")).toBeNull();
  });

  it("shows subscription rows as percents and sessions as spent / cap", () => {
    renderPanel();
    expect(screen.getByText("62%")).toBeInTheDocument();
    expect(screen.getByText("41%")).toBeInTheDocument();
    expect(screen.queryByText("$1.25")).toBeNull();
    expect(screen.getByText("$3.80")).toBeInTheDocument();
    expect(screen.getByText("$1.30")).toBeInTheDocument();
    const session = screen.getByText("acme-web").closest("[data-lub-row]");
    expect(session).not.toBeNull();
    const fill = session!.querySelector(".lub-meter-fill");
    expect(fill?.className).not.toMatch(/warn|muted/);
    expect(screen.getByText("· 接近上限")).toBeInTheDocument();
    const meter = within(session as HTMLElement).getByRole("meter");
    expect(meter).toHaveAttribute("data-near", "true");
    expect(meter.getAttribute("aria-label")).toContain("接近上限");
    expect(meter.getAttribute("title")).toBe("已用 76%，接近上限");
    expect(fill?.className).toContain("lub-meter-fill");
  });

  it("keeps the accent fill below the default 80% threshold", () => {
    renderPanel(designPanelFixture({ alertThresholdPct: 80 }));
    expect(screen.queryByText("· 接近上限")).toBeNull();
    const session = screen.getByText("acme-web").closest("[data-lub-row]");
    expect(session?.querySelector(".lub-meter-fill")?.className).not.toMatch(
      /warn/,
    );
  });

  it("adds the near caption at the default threshold without changing color", () => {
    const model = designPanelFixture({ alertThresholdPct: 80 });
    model.sessions![0].spentUsd = "4";
    renderPanel(model);
    const session = screen.getByText("acme-web").closest("[data-lub-row]");
    const fill = session?.querySelector(".lub-meter-fill");
    expect(fill?.className).not.toMatch(/warn|muted/);
    expect(screen.getByText("· 接近上限")).toBeInTheDocument();
    expect(within(session as HTMLElement).getByRole("meter")).toHaveAttribute(
      "title",
      "已用 80%，接近上限",
    );
  });

  it("opens the day-line tooltip from the keyboard", async () => {
    const user = userEvent.setup();
    renderPanel();
    const day = document.querySelector("[data-lub-marker='day']");
    expect(day).not.toBeNull();
    day!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    await user.click(day as HTMLElement);
    expect(await screen.findByText("到 $10 自动停")).toBeInTheDocument();
  });

  it("keeps cap-enforced copy in the row accessible name only", () => {
    renderPanel();
    const claude = screen.getByRole("meter", {
      name: "Claude Code，受上限管控",
    });
    expect(claude).toBeInTheDocument();
    const cursor = screen.getByRole("meter", { name: /Cursor，仅监控/ });
    expect(within(cursor).queryByText("仅监控")).toBeNull();
  });

  it("draws beta cut-off lines in the shared cap style, without a beta mark", () => {
    renderPanel(betaPanelFixture());
    const beta = document.querySelectorAll("[data-lub-marker='beta']");
    expect(beta).toHaveLength(2);
    for (const marker of beta) {
      expect(marker).toHaveAttribute("data-cap-ink", "day");
      expect(marker).toHaveClass("lub-cap");
      expect(marker.className).not.toMatch(/warn|orange|stop/);
    }
    expect(screen.queryByText("β")).toBeNull();
    expect(screen.queryByText("Beta")).toBeNull();
  });
});

describe("TrayPanelV5 stopped fixture", () => {
  it("keeps the cap line in the daily ink and labels stopped rows", async () => {
    const user = userEvent.setup();
    renderPanel(killedPanelFixture());
    const hit = document.querySelector("[data-lub-marker='hit']");
    const month = document.querySelector("[data-lub-marker='month']");
    expect(hit).not.toBeNull();
    expect(hit).toHaveAttribute("data-cap-ink", "day");
    expect(hit).toHaveClass("lub-cap");
    expect(hit?.className).not.toContain("lub-cap-month");
    expect(hit?.className).not.toMatch(/warn|orange|stop/);
    expect(month).toHaveClass("lub-cap-month");
    expect(month).toHaveAttribute("data-cap-ink", "month");

    const stopped = screen.getAllByText("已停止");
    expect(stopped.length).toBeGreaterThanOrEqual(3);
    expect(screen.getByText("仍在运行，62%")).toBeInTheDocument();
    expect(screen.getByText("已停止全部 API 花费")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "恢复 API 流量" }),
    ).toHaveAttribute("aria-disabled", "true");

    const today = screen.getByRole("meter", { name: /已停止/ });
    expect(today.querySelector(".lub-meter-fill")).toHaveClass(
      "lub-meter-fill-muted",
    );
    const claude = screen.getByRole("meter", {
      name: "Claude Code，受上限管控",
    });
    expect(claude.querySelector(".lub-meter-fill")).toHaveClass(
      "lub-meter-fill-muted",
    );

    await user.click(hit as HTMLElement);
    expect(
      await screen.findByText("已手动停止，点「恢复」继续"),
    ).toBeInTheDocument();
    expect(screen.queryByText("停止全部 API 花费")).toBeNull();
  });
});
