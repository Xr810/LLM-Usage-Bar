import { render, screen } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import {
  HEATMAP_MAX_WEEKS,
  TrendsPage,
  WEEKDAY_HOUR_FEATURE,
} from "./TrendsPage";

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  await i18n.changeLanguage("zh");
});

describe("TrendsPage", () => {
  it("shows 52 weeks and hides weekday-by-hour unless the feature is present", () => {
    const { rerender } = render(<TrendsPage />);
    expect(HEATMAP_MAX_WEEKS).toBe(52);
    expect(screen.getByText("近 52 周每日花费")).toBeInTheDocument();
    expect(screen.queryByText("星期与时段")).toBeNull();
    expect(screen.queryByText("PRO")).toBeNull();
    expect(screen.queryByText("近 26 周每日花费")).toBeNull();
    expect(document.querySelectorAll(".lub-heat-grid i")).toHaveLength(52 * 7);

    rerender(<TrendsPage features={[WEEKDAY_HOUR_FEATURE]} />);
    expect(screen.getByText("星期与时段")).toBeInTheDocument();
    expect(screen.getByText("高峰：周二 22–24 点")).toBeInTheDocument();
  });
});
