import { createRef } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderDailyBudgetField } from "./ProviderDailyBudgetField";

const mutation = vi.hoisted(() => ({
  mutateAsync: vi.fn(),
  isPending: false,
}));

vi.mock("@/lib/query/trayUsage", () => ({
  useSetProviderDailyBudget: () => mutation,
}));

function renderField(
  props: Partial<React.ComponentProps<typeof ProviderDailyBudgetField>> = {},
) {
  const onSave = props.onSave ?? vi.fn().mockResolvedValue(undefined);
  const view = render(
    <ProviderDailyBudgetField
      providerId="system-openai-api"
      providerName="OpenAI API"
      value="10"
      onSave={onSave}
      {...props}
    />,
  );
  return { ...view, onSave };
}

describe("ProviderDailyBudgetField", () => {
  const originalScrollIntoView = Element.prototype.scrollIntoView;

  beforeEach(() => {
    mutation.mutateAsync.mockReset().mockResolvedValue(undefined);
    mutation.isPending = false;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    if (originalScrollIntoView) {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    } else {
      delete (
        Element.prototype as Partial<Element> & {
          scrollIntoView?: Element["scrollIntoView"];
        }
      ).scrollIntoView;
    }
  });

  it("uses a real noValidate USD form and preserves the submitted decimal string", async () => {
    const user = userEvent.setup();
    const { onSave, container } = renderField();
    const form = container.querySelector("form");
    const input = screen.getByRole("spinbutton", {
      name: /openai api.*daily budget/i,
    });

    expect(form).toHaveAttribute("novalidate");
    expect(input).toHaveAttribute("type", "number");
    expect(input).toHaveAttribute("min", "0.000001");
    expect(input).toHaveAttribute("step", "0.01");
    expect(screen.getByText("USD")).toBeInTheDocument();

    fireEvent.change(input, { target: { value: "025.50" } });
    await user.click(screen.getByRole("button", { name: /save budget/i }));

    expect(onSave).toHaveBeenCalledWith("025.50");
    expect(input).toHaveValue(25.5);
  });

  it("clears a configured budget through the dedicated null command", async () => {
    const user = userEvent.setup();
    const { onSave } = renderField();

    await user.click(screen.getByRole("button", { name: /clear budget/i }));

    expect(onSave).toHaveBeenCalledWith(null);
    expect(
      screen.getByRole("spinbutton", { name: /openai api.*daily budget/i }),
    ).toHaveValue(null);
  });

  it.each(["", "0", "-1", "1e999"])(
    "rejects empty, non-finite, or non-positive budget %j",
    async (value) => {
      const user = userEvent.setup();
      const { onSave } = renderField({ value: null });
      const input = screen.getByRole("spinbutton", {
        name: /openai api.*daily budget/i,
      });
      fireEvent.change(input, { target: { value } });

      await user.click(screen.getByRole("button", { name: /save budget/i }));

      expect(screen.getByRole("alert")).toHaveTextContent(/positive amount/i);
      expect(onSave).not.toHaveBeenCalled();
    },
  );

  it("disables the input and both actions while pending", () => {
    renderField({ isPending: true });

    expect(screen.getByRole("spinbutton")).toBeDisabled();
    expect(screen.getByRole("button", { name: /save budget/i })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /clear budget/i }),
    ).toBeDisabled();
  });

  it("shows only a fixed localized error after backend failure and resets it after success", async () => {
    const user = userEvent.setup();
    const sentinel = "backend-secret-sentinel";
    const onSave = vi
      .fn()
      .mockRejectedValueOnce(new Error(sentinel))
      .mockResolvedValueOnce(undefined);
    renderField({ onSave });
    const input = screen.getByRole("spinbutton");

    await user.clear(input);
    await user.type(input, "12.34");
    await user.click(screen.getByRole("button", { name: /save budget/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /unable to update.*budget/i,
    );
    expect(document.body).not.toHaveTextContent(sentinel);

    await user.click(screen.getByRole("button", { name: /save budget/i }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(onSave).toHaveBeenLastCalledWith("12.34");
    expect(input).toHaveValue(12.34);
  });

  it("uses the built-in mutation when no override is supplied", async () => {
    const user = userEvent.setup();
    render(
      <ProviderDailyBudgetField
        providerId="system-openrouter-api"
        providerName="OpenRouter"
        value="8"
      />,
    );
    const input = screen.getByRole("spinbutton");
    await user.clear(input);
    await user.type(input, "9.75");
    await user.click(screen.getByRole("button", { name: /save budget/i }));

    expect(mutation.mutateAsync).toHaveBeenCalledWith({
      providerId: "system-openrouter-api",
      dailyBudgetUsd: "9.75",
    });
  });

  it("scrolls the wrapper to center, focuses the forwarded input, and handles a target once", async () => {
    const forwardedRef = createRef<HTMLInputElement>();
    const onTargetHandled = vi.fn();
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 1;
    });

    const { rerender } = render(
      <ProviderDailyBudgetField
        ref={forwardedRef}
        providerId="system-openai-api"
        providerName="OpenAI API"
        value="10"
        targeted
        onTargetHandled={onTargetHandled}
      />,
    );

    await waitFor(() => expect(forwardedRef.current).toHaveFocus());
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "center" });
    expect(onTargetHandled).toHaveBeenCalledOnce();
    expect(
      document.getElementById("provider-budget-system-openai-api"),
    ).not.toBeNull();

    rerender(
      <ProviderDailyBudgetField
        ref={forwardedRef}
        providerId="system-openai-api"
        providerName="OpenAI API"
        value="10"
        targeted
        onTargetHandled={onTargetHandled}
      />,
    );
    expect(onTargetHandled).toHaveBeenCalledOnce();
  });
});
