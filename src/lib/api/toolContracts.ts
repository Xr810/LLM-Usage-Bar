import { z } from "zod";

// These schemas describe the existing wire format, not a new runtime rejection
// policy. Rust serialization and these schemas share a fixture in contract tests.
export const toolVersionSchema = z.strictObject({
  name: z.string(),
  version: z.string().nullable(),
  latest_version: z.string().nullable(),
  error: z.string().nullable(),
  installed_but_broken: z.boolean(),
  env_type: z.enum(["windows", "wsl", "macos", "linux", "unknown"]),
  wsl_distro: z.string().nullable(),
});

export const wslShellPreferenceSchema = z.strictObject({
  wslShell: z.string().nullable().optional(),
  wslShellFlag: z.string().nullable().optional(),
});

export const toolInstallationSchema = z.strictObject({
  path: z.string(),
  version: z.string().nullable(),
  runnable: z.boolean(),
  error: z.string().nullable(),
  source: z.string(),
  is_path_default: z.boolean(),
});

export const toolInstallationReportSchema = z.strictObject({
  tool: z.string(),
  installs: z.array(toolInstallationSchema),
  is_conflict: z.boolean(),
  needs_confirmation: z.boolean(),
  command: z.string(),
  anchored: z.boolean(),
});

export type ToolVersion = z.infer<typeof toolVersionSchema>;
export type WslShellPreferenceInput = z.infer<typeof wslShellPreferenceSchema>;
export type ToolInstallation = z.infer<typeof toolInstallationSchema>;
export type ToolInstallationReport = z.infer<
  typeof toolInstallationReportSchema
>;
