import compatibilityManifestJson from "./productIdentityCompatibilityManifest.json";

export type OldIdentityOccurrenceKind =
  | "ownedRename"
  | "legacyReadOnly"
  | "externalWireStable"
  | "unsupportedPlatform";

export interface OldIdentityOccurrence {
  file: string;
  lineNumber: number;
  columnNumber: number;
  matchedText: string;
  context: string;
  kind: OldIdentityOccurrenceKind;
  reason: string;
}

interface RemovedOwnedIdentityLine {
  file: string;
  lineNumber: number;
  context: string;
  reason: string;
}

export interface OldIdentityMatch {
  columnNumber: number;
  matchedText: string;
}

export function findOldIdentityMatches(line: string): OldIdentityMatch[] {
  const normalizedLine = line.trim();
  return [...normalizedLine.matchAll(/cc[ _-]?switch/gi)].map((match) => ({
    columnNumber: (match.index ?? 0) + 1,
    matchedText: match[0],
  }));
}

const removedTask4OwnedLines: readonly RemovedOwnedIdentityLine[] = [
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 44,
    context: "CcSwitch,",
    reason: "prior writable current-product storage variant",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 483,
    context:
      'SkillStorageLocation::CcSwitch => get_app_config_dir().join("skills"),',
    reason: "prior writable current-product storage path arm",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 1168,
    context:
      'SkillStorageLocation::CcSwitch => get_app_config_dir().join("skills"),',
    reason: "prior storage migration target arm",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 1402,
    context: 'scan_sources.push((ssot_dir, "cc-switch".to_string()));',
    reason: "prior transient current-product scan label",
  },
  {
    file: "src-tauri/src/services/skill.rs",
    lineNumber: 1472,
    context:
      'search_sources.push((ssot_dir.clone(), "cc-switch".to_string()));',
    reason: "prior transient current-product import label",
  },
  {
    file: "src-tauri/src/settings.rs",
    lineNumber: 448,
    context:
      "/// Skill 存储位置：cc_switch（默认）或 unified（~/.agents/skills/）",
    reason: "prior current settings wire documentation",
  },
  {
    file: "src/types.ts",
    lineNumber: 235,
    context: 'export type SkillStorageLocation = "cc_switch" | "unified";',
    reason: "prior writable frontend storage wire value",
  },
  // The src/lib/api/skills.ts entry was dropped with that module: it asserted
  // the file no longer carries a legacy literal, and the file is now gone.
  {
    file: "src/lib/schemas/settings.ts",
    lineNumber: 38,
    context:
      'skillStorageLocation: z.enum(["cc_switch", "unified"]).optional(),',
    reason: "prior current settings write schema",
  },
  // The two SkillStorageLocationSettings.tsx entries were dropped when that
  // component was removed. Each asserted the file no longer carries a legacy
  // literal; with the file itself gone there is nothing left to assert, and
  // the check cannot read a path that does not exist.
];

function removedOwnedOccurrence(
  line: RemovedOwnedIdentityLine,
): OldIdentityOccurrence {
  const matches = findOldIdentityMatches(line.context);
  if (matches.length !== 1) {
    throw new Error(
      `${line.file}:${line.lineNumber} removed-owned context must contain one exact match`,
    );
  }
  return {
    ...line,
    ...matches[0],
    kind: "ownedRename",
  };
}

const preservedKinds = new Set<OldIdentityOccurrenceKind>([
  "legacyReadOnly",
  "externalWireStable",
  "unsupportedPlatform",
]);

function validatedCompatibilityManifest(
  value: unknown,
): OldIdentityOccurrence[] {
  if (!Array.isArray(value)) {
    throw new Error("product identity compatibility manifest must be an array");
  }

  const seen = new Set<string>();
  return value.map((candidate, index) => {
    if (!candidate || typeof candidate !== "object") {
      throw new Error(
        `compatibility manifest entry ${index} must be an object`,
      );
    }
    const entry = candidate as Record<string, unknown>;
    const file = entry.file;
    const lineNumber = entry.lineNumber;
    const columnNumber = entry.columnNumber;
    const matchedText = entry.matchedText;
    const context = entry.context;
    const kind = entry.kind;
    const reason = entry.reason;

    if (
      typeof file !== "string" ||
      file.startsWith("/") ||
      file.includes("..") ||
      typeof lineNumber !== "number" ||
      !Number.isSafeInteger(lineNumber) ||
      lineNumber < 1 ||
      typeof columnNumber !== "number" ||
      !Number.isSafeInteger(columnNumber) ||
      columnNumber < 1 ||
      typeof matchedText !== "string" ||
      typeof context !== "string" ||
      typeof kind !== "string" ||
      !preservedKinds.has(kind as OldIdentityOccurrenceKind) ||
      typeof reason !== "string" ||
      reason.trim() === ""
    ) {
      throw new Error(`compatibility manifest entry ${index} is malformed`);
    }

    const exactMatch = findOldIdentityMatches(context).find(
      (match) =>
        match.columnNumber === columnNumber &&
        match.matchedText === matchedText,
    );
    if (!exactMatch) {
      throw new Error(
        `${file}:${lineNumber}:${columnNumber} does not pin an exact old identity match`,
      );
    }

    const key = `${file}:${lineNumber}:${columnNumber}`;
    if (seen.has(key)) {
      throw new Error(`duplicate compatibility manifest entry ${key}`);
    }
    seen.add(key);

    return {
      file,
      lineNumber,
      columnNumber,
      matchedText,
      context,
      kind: kind as OldIdentityOccurrenceKind,
      reason,
    };
  });
}

const removedOwnedOccurrences = removedTask4OwnedLines.map(
  removedOwnedOccurrence,
);
const compatibilityOccurrences = validatedCompatibilityManifest(
  compatibilityManifestJson,
);

export const oldIdentityOccurrences: readonly OldIdentityOccurrence[] = [
  ...removedOwnedOccurrences,
  ...compatibilityOccurrences,
];

export function classifyOldIdentityOccurrence(
  file: string,
  lineNumber: number,
  columnNumber: number,
  matchedText: string,
  line: string,
): OldIdentityOccurrence | undefined {
  const normalizedFile = file.replace(/\\/g, "/").replace(/^\.\//, "");
  const normalizedLine = line.trim();
  return oldIdentityOccurrences.find(
    (entry) =>
      entry.file === normalizedFile &&
      entry.lineNumber === lineNumber &&
      entry.columnNumber === columnNumber &&
      entry.matchedText === matchedText &&
      entry.context === normalizedLine,
  );
}
