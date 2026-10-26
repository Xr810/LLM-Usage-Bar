import { UpgradePanel } from "./UpgradePanel";

/**
 * Opens the unified upgrade panel. Trial and purchase stay in the hidden slot.
 */
export function UpsellPanel({ onClose }: { onClose: () => void }) {
  return <UpgradePanel onClose={onClose} upsellEntry />;
}
