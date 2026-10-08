# macOS / Windows native acceptance — 2026-10-08

This documentation-only branch preserves sanitized native acceptance results for
[aa828178c59bfd7db9358ef5df912e369602f99d](https://github.com/Xr810/LLM-Usage-Bar/commit/aa828178c59bfd7db9358ef5df912e369602f99d)
(3.17.0), whose direct parent is the watcher fix
[9c48788d406e86e234c4d46e5fa3f91bfad22e97](https://github.com/Xr810/LLM-Usage-Bar/commit/9c48788d406e86e234c4d46e5fa3f91bfad22e97).
The coordinator verified that exact source checkout; it did not re-execute native
tests in its Linux Orb. Reports came from the original platform runners and were
reviewed before publication. These are historical results, not proof that later
repair commits pass.

**Overall acceptance remains NOT PASSING.** Neither successful packaging nor
focused tests cancel out original failures. Diagnostic skips and workarounds are
explicitly disclosed. No repair code is included, pushed, merged or released.

## Platform reports and evidence provenance

- [Windows report](windows.md): commands, 19 library failures, 6 integration
  failures (5 mutex-poison cascades), LF/CRLF diagnosis, installer property
  transitions, installed-app behavior, limits and package checksums.
- [macOS report](macos-runner-aa828178.md): commands, default strip build failure, frontend
  concurrency failure, installed-app behavior, credential availability and
  initialization observations, limits and package checksums.
- Original [Windows runner thread](https://ampcode.com/threads/T-01a11bf4-b63e-74af-a5ab-e451b45d515c)
  and [Mac runner thread](https://ampcode.com/threads/T-01a11bf4-ad56-7654-9c3d-1c33db239277).
- [Coordination thread](https://ampcode.com/threads/T-01a11c3f-cfd4-73ac-8c91-e0ec2c774cb0).

Evidence basenames in these reports identify private runner files; they are not
download links or files present in an Orb. Request a reviewed, sanitized excerpt
or driver from the corresponding original thread when needed. This branch does
not contain databases, credential values, personal logs, screenshots, installers
or evidence ZIPs. Installer hashes preserve provenance without publishing binaries.

## Independent root-cause tracking and repair ownership

Failure counts are not issue counts. Each issue has its own independent Orb;
cross-platform shared lifecycle behavior is tracked once. Unknown causes remain
explicit investigations, not confirmed production failures.

| Issue / scope                                                                                                                           | Independent repair Orb                                                               |
| --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| [#59 Windows CLI test contract](https://github.com/Xr810/LLM-Usage-Bar/issues/59) — six expectations vs intentional Codex policy        | [CLI Orb](https://ampcode.com/threads/T-01a11c4c-e3e8-72ec-bd72-5f4a948ec4a4)        |
| [#60 Windows identity migration](https://github.com/Xr810/LLM-Usage-Bar/issues/60) — retirement/WAL and app_config_load mutex cascade   | [Identity Orb](https://ampcode.com/threads/T-01a11c4c-f033-77d8-9c17-7d8ff8c98f85)   |
| [#61 Windows cursor/schema](https://github.com/Xr810/LLM-Usage-Bar/issues/61) — classification and rollback-test contract               | [Cursor Orb](https://ampcode.com/threads/T-01a11c4c-fac8-778d-b447-9c370eb99c84)     |
| [#62 175.66s in-flight quit](https://github.com/Xr810/LLM-Usage-Bar/issues/62) — common lifecycle ownership, Windows reproduction       | [Shutdown Orb](https://ampcode.com/threads/T-01a11c4d-03e6-77ed-841c-709e547ef2e3)   |
| [#63 LF/CRLF cache fixture](https://github.com/Xr810/LLM-Usage-Bar/issues/63)                                                           | [Cache Orb](https://ampcode.com/threads/T-01a11c4d-0b81-769f-bce0-cb6883337857)      |
| [#64 Mac default universal strip](https://github.com/Xr810/LLM-Usage-Bar/issues/64) — strip=none is only a workaround                   | [Release Orb](https://ampcode.com/threads/T-01a11c4d-15bf-756e-9c4f-be41bc12261d)    |
| [#65 Mac frontend 5s timeout](https://github.com/Xr810/LLM-Usage-Bar/issues/65) — one-worker pass is not default-concurrency acceptance | [Frontend Orb](https://ampcode.com/threads/T-01a11c4d-1d96-7426-9476-1fbdfa18fc4b)   |
| [#66 Mac isolated credential startup](https://github.com/Xr810/LLM-Usage-Bar/issues/66) — environment/product distinction unresolved    | [Credential Orb](https://ampcode.com/threads/T-01a11c4d-27a1-73c9-b0da-9760b16cb3e4) |
| [#67 MSI explicit INSTALLDIR](https://github.com/Xr810/LLM-Usage-Bar/issues/67) — AppSearch override confirmed in native log            | [MSI Orb](https://ampcode.com/threads/T-01a11c4e-0d14-7109-9b85-90fb8279cb0e)        |

The CLI source deliberately excludes Codex self-update; do not restore it solely
to satisfy obsolete assertions. Cursor tests include Unix paths whose Windows
absolute-path semantics differ; investigate before claiming a production rollback
bug. Identity migration fails closed to prevent a data-loss race; a safe repair
must preserve that invariant. The credential issue records observed degraded
startup, not a proven production Keychain defect. MSI no-prior-key, dual-key and
interactive matrices were not executed and are not claimed failures.

## Acceptance limitations — not confirmed code defects

- Mac accessibility permission was unavailable; tray interaction and the managed
  tray-cleanup path were not established by AppleEvent exit. AppleEvent normal exit
  is not a substitute for those scenarios.
- Mac package was ad-hoc signed / not notarized; Windows packages were unsigned.
  Missing signing credentials, Gatekeeper/SmartScreen distribution trust, Intel
  hardware absence and unexecuted scenarios are limits, not new bug reports.
- Intel slice build verification does not establish Intel-machine runtime success.
- Real provider accounts, online quota, precise unowned startup-task exit races,
  and permanently blocked I/O were not validated.
- Mac PATH loss and incorrect harness session-id lookup were diagnosed and
  corrected as test/environment mistakes; do not count them as product failures.
- Windows frontend success used `--maxWorkers=2`, not default concurrency.
- The 175.6648448s Windows value is an observed elapsed upper bound, not a precise
  internal shutdown phase duration. Existing 5s unit-test limits are not a product
  force-kill deadline. Data survived natural exit; responsiveness did not pass.

## Follow-up protocol

Repair Orbs own implementation and tests without mixing changes in the coordinator.
Coordinate shared-file boundaries before modifying them. Transfer exact unpushed
patches through thread file tools; merely citing a local commit does not make it
available to another runner. Original runners should serialize expensive native
checks, use disposable data, and record the exact patch commit and commands.
Linux checks are supplemental, never replacements for Mac/Windows native acceptance.

Each repair Orb must report implementation, verification and remaining limits to
the requesting Puck thread and coordinator. Local repair commits may be retained,
but repair pushes, main merges, package publication and releases are not authorized.
Only this test-record documentation branch is authorized to be pushed.
