import { invoke } from "@tauri-apps/api/core";
import type { LicenseStatus } from "@/types/license";

export const getLicenseStatus = () =>
  invoke<LicenseStatus>("get_license_status");
