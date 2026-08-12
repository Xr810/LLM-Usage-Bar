# Documentation map

Where each kind of document lives. The rule is **live vs. historical**: anything
under `archive/` is a record of what happened and is never updated; everything
else is expected to be correct today.

> The single entry point for anyone picking up work on this repo is
> [`HANDOFF.md`](../HANDOFF.md) at the repository root — read it first.

## Live

| Folder | What's in it |
| --- | --- |
| [`user-manual/`](user-manual/) | End-user manual, in `en` / `ja` / `zh`. Shared screenshots in `user-manual/assets/`. |
| [`guides/`](guides/) | Task-focused how-tos (Codex routing, auth preservation, proxy, cargo cache). Images in `images/`. |
| [`release-notes/`](release-notes/) | Per-version release notes, `vX.Y.Z-{en,ja,zh}.md`. |
| [`design/`](design/) | Design specs and PRDs that still describe intended behaviour, including the undecided [tech-route review](design/tech-route-review-2026-08-12.md). |
| [`testing/`](testing/) | Manual acceptance material: the usage-dashboard runbook and the deep-link test page. |
| [`images/`](images/) | Image store for `guides/` and archived design docs. |

## Historical

| Folder | What's in it |
| --- | --- |
| [`archive/plans/`](archive/plans/) | Dated implementation plans from the superpowers/SDD era. Archaeology only. |
| [`archive/task-state/`](archive/task-state/) | Superseded task-state ledgers. All of it was folded into `HANDOFF.md` on 2026-08-07. |
| `archive/*.md` | One-off records: the 2026-07-17 frontend redesign and the 2026-07-19 design QA. |

## Conventions

- Dated documents use `YYYY-MM-DD-` prefixes; the older suffix style survives in
  `archive/` and is not worth churning.
- Translated documents share a base name and differ only by an `-en` / `-ja` /
  `-zh` suffix.
- New work does **not** get a new top-level doc — append to `HANDOFF.md`.
