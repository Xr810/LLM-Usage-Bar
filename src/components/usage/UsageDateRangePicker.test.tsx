import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { UsageDateRangePicker } from "./UsageDateRangePicker";

describe("UsageDateRangePicker", () => {
  it.each(["10:00", "09:59"])(
    "rejects an end time of %s at or before the start and allows correction",
    async (endTime) => {
      const user = userEvent.setup();
      const onApply = vi.fn();
      const start = new Date(2026, 9, 15, 10).getTime() / 1000;
      render(
        <UsageDateRangePicker
          selection={{
            preset: "custom",
            customStartDate: start,
            customEndDate: start + 3600,
          }}
          onApply={onApply}
          triggerLabel="Custom range"
        />,
      );
      await user.click(screen.getByRole("button", { name: "Custom range" }));
      const endInput = document.querySelectorAll('input[type="time"]')[1];
      fireEvent.change(endInput, { target: { value: endTime } });
      await user.click(screen.getByRole("button", { name: "common.confirm" }));

      expect(onApply).not.toHaveBeenCalled();
      expect(screen.getByRole("alert")).toHaveTextContent(
        "开始时间必须早于结束时间",
      );
      expect(screen.getByRole("dialog")).toBeVisible();

      fireEvent.change(endInput, { target: { value: "10:01" } });
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "common.confirm" }));
      expect(onApply).toHaveBeenCalledExactlyOnceWith({
        preset: "custom",
        customStartDate: start,
        customEndDate: start + 60,
        liveEndTime: false,
      });
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    },
  );

  it("offers a 1 year shortcut and explains that longer custom ranges are supported", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();

    render(
      <UsageDateRangePicker
        selection={{ preset: "30d" }}
        onApply={onApply}
        triggerLabel="Custom range"
      />,
    );

    await user.click(screen.getByRole("button", { name: "Custom range" }));

    expect(
      await screen.findByText("支持日期与时间，可选择超过 1 年的范围"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "1 year" }));

    expect(onApply).toHaveBeenCalledWith({ preset: "1y" });
  });
});
