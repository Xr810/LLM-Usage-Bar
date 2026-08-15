import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import type { UsageProviderView } from "@/types/usageDashboard";
import { SystemProviderPicker } from "./SystemProviderPicker";

function provider(id: string, name: string, enabled = false) {
  return {
    id,
    name,
    systemPresetKey: id,
    billingKind: "metered",
    enabled,
    canonicalEndpoint: `https://${id}.example/v1`,
  } as unknown as UsageProviderView;
}

const providers = [
  provider("openai-api", "OpenAI API", true),
  provider("kimi-api", "Kimi / Moonshot API"),
  provider("cerebras-api", "Cerebras API"),
];

function renderInDialog(onToggle = vi.fn()) {
  render(
    <Dialog open>
      <DialogContent>
        <SystemProviderPicker providers={providers} onToggle={onToggle} />
      </DialogContent>
    </Dialog>,
  );
  return onToggle;
}

describe("SystemProviderPicker", () => {
  it("renders its list inside the dialog that contains it", () => {
    renderInDialog();
    fireEvent.click(
      screen.getByRole("button", { name: /Add a built-in Provider/ }),
    );

    // A modal Dialog wraps its content in react-remove-scroll, which blocks
    // wheel events outside that subtree. Portalled to document.body the list
    // renders correctly and refuses to scroll, which is not something CSS can
    // fix — so containment is the property worth pinning.
    // Radix gives its own popover `role="dialog"` too, so target the settings
    // surface by the attribute only DialogContent sets.
    const dialog = document.querySelector("[data-surface-variant]");
    const option = screen.getByRole("checkbox", { name: "Cerebras API" });
    expect(dialog).not.toBeNull();
    expect(dialog!.contains(option)).toBe(true);
  });

  it("shows the whole catalogue and reports how much of it is picked", () => {
    renderInDialog();
    fireEvent.click(
      screen.getByRole("button", { name: /Add a built-in Provider/ }),
    );

    expect(screen.getAllByRole("checkbox")).toHaveLength(providers.length);
    expect(screen.getByRole("checkbox", { name: "OpenAI API" })).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "Cerebras API" }),
    ).not.toBeChecked();
  });

  it("reports both directions of a toggle", () => {
    const onToggle = renderInDialog();
    fireEvent.click(
      screen.getByRole("button", { name: /Add a built-in Provider/ }),
    );

    fireEvent.click(screen.getByRole("checkbox", { name: "Cerebras API" }));
    expect(onToggle).toHaveBeenLastCalledWith(providers[2], true);

    fireEvent.click(screen.getByRole("checkbox", { name: "OpenAI API" }));
    expect(onToggle).toHaveBeenLastCalledWith(providers[0], false);
  });
});
