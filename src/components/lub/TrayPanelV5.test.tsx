import { readFileSync } from "node:fs";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { CAP_LINE_RATIO } from "./capMath";
import { formatResetInstant } from "./money";
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

  it("reads a beta subscription row as cap-enforced at the threshold", async () => {
    const user = userEvent.setup();
    renderPanel(betaPanelFixture());
    const meter = screen.getByRole("meter", {
      name: /Claude Max，受上限管控/,
    });
    expect(meter).toHaveAccessibleName(/订阅截断 Beta：到 90%/);
    expect(meter.querySelectorAll("[data-lub-marker]")).toHaveLength(1);
    const cursor = screen.getByRole("meter", { name: /Cursor，仅监控/ });
    expect(cursor.querySelector("[data-lub-marker]")).toBeNull();
    await user.click(
      meter.querySelector("[data-lub-marker='beta']") as HTMLElement,
    );
    expect(
      await screen.findByText("到 90% 自动停（Beta）"),
    ).toBeInTheDocument();
  });
});

describe("TrayPanelV5 stale badge", () => {
  it("shows the existing stale label without a new key", () => {
    renderPanel(designPanelFixture({ stale: true }));
    const badge = screen.getByRole("status");
    expect(badge).toHaveTextContent("数据已过期");
    expect(badge).toHaveClass("lub-stale");
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

describe("TrayPanelV5 session groups", () => {
  it("labels an unattributed row and the unparsed-amount hint", () => {
    const model = designPanelFixture();
    model.sessions = [
      {
        id: "loose",
        alias: "",
        group: "unattributed",
        clientKey: null,
        clientLabel: null,
        spentUsd: "1.00",
        capUsd: "5",
      },
    ];
    model.invalidCostRows = 2;
    renderPanel(model);
    expect(screen.getByText("未归属")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "有 2 条记录金额无法解析",
    );
    expect(screen.queryByText("0%")).toBeNull();
  });
});

describe("TrayPanelV5 free over cap", () => {
  function overFixture() {
    return designPanelFixture({
      state: "over",
      todayUsd: "12.00",
      dailyCapUsd: "10",
    });
  }

  it("alerts without kill, compare, or a red overflow segment", async () => {
    const user = userEvent.setup();
    renderPanel(overFixture());
    expect(screen.getByText(/已超出今日上限 \$10/)).toBeInTheDocument();
    expect(
      screen.getByText("只提醒，不会暂停。API 请求照常发送。"),
    ).toBeInTheDocument();
    expect(document.querySelector(".lub-status-warn")).toHaveTextContent(
      "超出上限",
    );
    expect(document.querySelector(".lub-dot-over")).not.toBeNull();
    expect(screen.queryByText("比较 Free 与 Pro")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "停止全部 API 花费" }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /放行/ })).toBeNull();
    expect(screen.queryByText("已暂停")).toBeNull();
    expect(document.querySelector("[data-slot='community-allow']")).toBeNull();
    expect(document.querySelector(".lub-meter-overflow")).toBeNull();
    expect(screen.queryByText(/浅色为月上限/)).toBeNull();
    expect(screen.queryByText(/重新计算/)).toBeNull();

    const day = document.querySelector("[data-lub-marker='day']");
    expect(day).toHaveAttribute("data-cap-ink", "day");
    expect(day?.className ?? "").not.toMatch(/warn/);
    const hero = document.querySelector(".lub-hero .lub-meter");
    expect(hero?.getAttribute("aria-label") ?? "").toContain("已超出");
    const fill = hero?.querySelector(".lub-meter-fill");
    expect(fill).toHaveStyle({ width: "100%" });
    expect(fill?.className ?? "").not.toMatch(/warn/);

    const warnNodes = document.querySelectorAll("[class*='warn']");
    expect(warnNodes.length).toBeGreaterThan(0);
    for (const node of warnNodes) {
      expect(node.className).toMatch(/lub-banner-warn|lub-status-warn/);
    }

    await user.click(day as HTMLElement);
    expect(
      await screen.findByText("日上限 $10，只提醒、不停"),
    ).toBeInTheDocument();
    expect(screen.queryByText("到 $10 自动停")).toBeNull();
  });

  it("fills the community slot with disabled allow buttons and no compare link", () => {
    render(
      <TrayPanelV5
        model={overFixture()}
        locale="zh-CN"
        build="community"
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    const slot = document.querySelector("[data-slot='community-allow']");
    expect(slot).not.toBeNull();
    expect(slot?.querySelectorAll("button")).toHaveLength(2);
    expect(slot?.querySelector("a, [href]")).toBeNull();
    expect(screen.getByText("社区构建")).toBeInTheDocument();
    expect(screen.queryByText("比较 Free 与 Pro")).toBeNull();
    expect(screen.queryByRole("button", { name: /停止全部/ })).toBeNull();
  });

  it("shows the compare line only when upsellEntry is on", () => {
    const { container } = render(
      <TrayPanelV5
        model={overFixture()}
        locale="zh-CN"
        upsellEntry
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    expect(
      screen.getByRole("button", { name: "比较 Free 与 Pro" }),
    ).toBeInTheDocument();
    expect(screen.getByText("想到上限就自动停？")).toBeInTheDocument();
    expect(container.querySelector("a, [href]")).toBeNull();
  });

  it("shows the reset caption only from a row nextResetAt", () => {
    const instant = "2026-10-27T16:30:00.000Z";
    const missing = overFixture();
    missing.sources = missing.sources.map((source) => ({
      ...source,
      nextResetAt: null,
    }));
    const { unmount } = renderPanel(missing);
    expect(screen.queryByText(/重新计算/)).toBeNull();
    unmount();

    const known = overFixture();
    known.sources = known.sources.map((source, index) =>
      index === 0 ? { ...source, nextResetAt: instant } : source,
    );
    renderPanel(known);
    expect(screen.getByText(/重新计算/)).toHaveTextContent(
      formatResetInstant(instant, "zh-CN") ?? "",
    );
  });

  it("hides the reset caption when row reset times disagree or are invalid", () => {
    const model = overFixture();
    model.sources = model.sources.map((source, index) => ({
      ...source,
      nextResetAt:
        index === 0
          ? "2026-10-27T16:30:00.000Z"
          : index === 1
            ? "not-a-time"
            : "2026-10-28T01:00:00.000Z",
    }));
    renderPanel(model);
    expect(screen.queryByText(/重新计算/)).toBeNull();
  });

  it("shows the codex websocket note only from a row note code", () => {
    const absent = designPanelFixture();
    absent.sources = absent.sources.map((source) => ({
      ...source,
      noteCodes: null,
    }));
    const { unmount } = renderPanel(absent);
    expect(screen.queryByText(/Codex 订阅未走代理/)).toBeNull();
    unmount();

    const other = designPanelFixture();
    other.sources = other.sources.map((source, index) =>
      index === 0 ? { ...source, noteCodes: ["stale"] } : source,
    );
    const rendered = renderPanel(other);
    expect(screen.queryByText(/Codex 订阅未走代理/)).toBeNull();
    rendered.unmount();

    const model = designPanelFixture();
    model.sources = model.sources.map((source, index) =>
      index === 3 ? { ...source, noteCodes: ["codex_websocket"] } : source,
    );
    renderPanel(model);
    expect(screen.getByText(/Codex 订阅未走代理/)).toBeInTheDocument();
  });
});

describe("lub warn color", () => {
  it("applies --lub-warn only to the banner, status label, and dot", () => {
    const css = readFileSync("src/styles/lub-panel.css", "utf8");
    const rules = css.match(/[^{}]+\{[^{}]+\}/g) ?? [];
    const warnRules = rules.filter((rule) => rule.includes("--lub-warn"));
    expect(warnRules.length).toBeGreaterThan(0);
    for (const rule of warnRules) {
      expect(rule).toMatch(
        /lub-dot-warn|lub-dot-over|lub-status-warn|lub-banner-warn/,
      );
    }
  });
});

describe("TrayPanelV5 license lapse", () => {
  it("keeps Lift all blocks enabled on the gray bar", () => {
    renderPanel(designPanelFixture({ licenseLapsed: true }));
    const bar = screen.getByRole("status");
    expect(bar).toHaveClass("lub-license-bar");
    expect(bar.className).not.toMatch(/warn|stop/);
    const lift = screen.getByRole("button", { name: "解除全部阻断" });
    expect(lift).toBeEnabled();
    expect(lift).not.toHaveAttribute("aria-disabled");
  });
});
