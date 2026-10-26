import { render, screen } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { CapsLegendWindow } from "./LegendCard";

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  await i18n.changeLanguage("zh");
});

describe("caps legend", () => {
  it("lists cap-enforced and monitor-only with one shared line style", () => {
    render(<CapsLegendWindow />);
    const card = screen.getByTestId("legend-card");
    expect(card).toHaveTextContent("受上限管控");
    expect(card).toHaveTextContent("仅监控");
    expect(card).not.toHaveTextContent("显示文字标签");
    expect(card).not.toHaveTextContent("β");
    expect(card).not.toHaveTextContent("橙色");
    expect(card).not.toHaveTextContent("只统计");

    const rows = card.querySelectorAll("[data-legend-row]");
    expect(rows).toHaveLength(4);
    const markers = card.querySelectorAll("[data-lub-marker]");
    expect(markers).toHaveLength(3);
    expect(markers[0]).toHaveAttribute("data-cap-ink", "day");
    expect(markers[1]).toHaveAttribute("data-cap-ink", "month");
    expect(markers[1]).toHaveClass("lub-cap-month");
    expect(markers[2]).toHaveAttribute("data-lub-marker", "beta");
    expect(markers[2]).toHaveAttribute("data-cap-ink", "day");
    expect(markers[2]).toHaveClass("lub-cap");
    expect(markers[2].className).not.toMatch(/warn|orange|stop/);
    expect(rows[2].querySelector("[data-lub-marker]")).toBeNull();
  });
});
