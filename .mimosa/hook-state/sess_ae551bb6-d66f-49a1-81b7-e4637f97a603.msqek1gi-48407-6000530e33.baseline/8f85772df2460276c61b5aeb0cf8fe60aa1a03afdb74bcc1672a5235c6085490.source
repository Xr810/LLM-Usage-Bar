import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { UsageDateRangePicker } from "./UsageDateRangePicker";

describe("UsageDateRangePicker", () => {
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
