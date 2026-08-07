# Official Pricing Refresh Design

**Date:** 2026-08-03
**Status:** Approved, not yet implemented
**Replaces:** the manual global pricing editor (`PricingConfigPanel` /
`PricingEditModal` / `ModelsDevPickerDialog`), which has been unreachable from
the renderer entry point since the provider-only redesign.

## Why

`model_pricing` is the official reference catalogue. Two things read it:

1. subscription accounts, whose "equivalent API cost" is **always** valued at
   official list prices;
2. metered accounts for models the user has not priced themselves.

Today it is populated only by a hardcoded seed in `schema.rs` that goes stale
with every vendor price change, and nothing in the running app can update it.
The catalogue should refresh itself from a published source instead.

## Source

`https://models.dev/api.json` — ~3.4 MB, 179 providers.

```jsonc
{
  "<providerId>": {
    "id": "anthropic",
    "name": "Anthropic",
    "models": {
      "<modelId>": {
        "id": "claude-sonnet-4-5",
        "name": "Claude Sonnet 4.5",
        "release_date": "2025-09-29",
        "cost": { "input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75 }
      }
    }
  }
}
```

`cost` values are USD per million tokens, matching `model_pricing`'s four
columns exactly. Some entries omit `cache_read` / `cache_write`; some carry an
extra `tiers` / `context_over_200k` block that this import ignores.

## First-party filter — the load-bearing rule

The same model appears under many providers with **different** prices. For
`claude-sonnet-4-5`: `anthropic`, `azure`, `neon`, `llmgateway` and others all
report $3/$15, but `venice` reports $3.75/$18.75 and `302ai` omits the cache
fields entirely.

An unfiltered import therefore picks a reseller's markup and calls it the
official price. Only the model's own vendor may populate this catalogue:

| Model family prefix | Accepted provider id |
| --- | --- |
| `claude-` | `anthropic` |
| `gpt-`, `o1`, `o3`, `o4`, `o5`, `codex-` | `openai` |
| `gemini-` | `google` |
| `deepseek-` | `deepseek` |
| `qwen-` | `alibaba` |
| `kimi-`, `moonshot-` | `moonshotai` |
| `glm-` | `zhipuai` |
| `grok-` | `xai` |
| `mistral-`, `codestral-` | `mistral` |

Anything from a provider not paired with the model's family is **skipped, not
imported**. Resellers' rates belong in `provider_model_pricing`, which the user
sets per account — never here. Verify each provider id against the live
document before hardcoding it; ids are lowercase slugs and a few differ from
the vendor's brand name.

## Model id normalization

Reuse the existing rule, do not re-invent it. Ids must be normalized exactly as
`clean_model_id_for_pricing` in `services/usage_stats.rs` does — last path
segment, drop anything after `:`, `@` to `-`, lowercase, strip the `[1m]`
marker — otherwise stored rows can never be matched by the lookup that reads
them. The now-deleted `ModelsDevPickerDialog` carried a TypeScript mirror of
this rule; the backend implementation is the authority.

models.dev publishes both family ids (`claude-sonnet-4-5`) and dated ids
(`claude-sonnet-4-5-20250929`). Import both. The lookup's date-suffix stripping
and prefix matching already handle either direction, and having both stored
makes matching exact rather than inferred.

## Implementation

**Backend, not renderer.** The existing dialog called `fetch()` straight from
the renderer. The refresh must run in Rust instead, so it works with the window
closed, goes through the shared `crate::http_client` (and therefore the app's
proxy and TLS settings), and is not subject to renderer network policy.

New `services/official_pricing.rs`:

- `refresh_official_pricing(db) -> Result<RefreshOutcome, AppError>` — fetch,
  parse, filter, normalize, upsert into `model_pricing`.
- `RefreshOutcome { fetched_at, models_imported, models_skipped, source_url }`.
- Upsert, never `DELETE` first: a failed or partial fetch must not leave the
  catalogue empty. A model that disappears upstream keeps its last known price.
- Persist `official_pricing_last_refresh_at` in `settings` so the UI can show
  freshness and the scheduler knows when it last ran.

Triggers:

- **Manual** — a Tauri command behind a "refresh now" button.
- **Periodic** — on startup if the last refresh is older than 7 days, then
  daily. Follow the existing quota scheduler's backoff shape
  (`usage/quota.rs`, `quota_retry_migration`) rather than inventing another.

Failure is non-fatal and must never block startup: log, keep the stored prices,
surface the staleness in the UI.

## User interface

One row in Settings, next to the API budget controls: last refresh time,
imported-model count, a refresh button, and the error state when the last
attempt failed. No per-model editing UI returns — the catalogue is now
machine-maintained, and per-account rates are edited in
`ProviderModelPricingSection`.

## Boundaries

- Never write `provider_model_pricing` from this import. That table is the
  user's own purchase price and is authoritative over anything fetched.
- Never change `usage_events`. Prices resolve at ingest; a refresh is not
  retroactive, consistent with the v20 decision.
- Keep the `schema.rs` seed as the offline fallback for a first run with no
  network.
