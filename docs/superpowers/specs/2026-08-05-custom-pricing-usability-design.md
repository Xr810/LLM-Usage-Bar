# Custom per-Provider pricing: partial prices and model discovery

Status: agreed, not yet implemented.

Two changes to the custom price editor on a metered Provider card
(`ProviderModelPricingSection`). Both come from the same complaint: the form
makes the user supply more than they know, and silently accepts a plausible
wrong answer when they don't.

## 1. A blank rate means "use the official price", not zero

Today the four rates are `TEXT NOT NULL`. Input and output reject a blank at
both layers, and the two cache fields are pre-filled with `0`, which is a legal
value and is stored as written.

For a relay that resells an OpenAI-style model, `0` is not a neutral default —
it is a claim that cached tokens are free. The cost path deducts the cached
prefix from billable input before applying the input rate, so a cache rate of
zero removes those tokens from the bill entirely. Codex traffic is ~96% cache,
so on real data the input side comes out about 7x low:

| cache rate | input-side cost of one 149,510-token turn (147,200 cached) |
| --- | --- |
| `0` (today's default) | $0.0116 |
| $0.50/M (~10% of input) | $0.085 |

Nothing on screen distinguishes the two. This is the same failure mode as the
cache-semantics bug fixed in `b0216feea`: a reasonable-looking default producing
a reasonable-looking number that is wrong.

**Design.** A rate the user leaves blank falls back to the official catalogue
entry for that model, per field. A relay may charge its own input rate and pass
output through at list price; that should be expressible by filling one box.

- `provider_model_pricing`'s four rate columns become nullable (schema v23).
- `canonicalize_price` maps an empty string to `None` rather than rejecting it.
- Price lookup merges the user's row over the official row field by field.
- The form shows the official rate as each field's placeholder, so a blank box
  states what it will use.

**When neither exists.** If a field is blank and the official catalogue has no
entry for that model, that rate has no value and the event is priced
`unavailable`. It must not fall back to zero — an event that reads as free is
worse than one that reads as unpriced, because only the second is visible as a
gap.

## 2. The model ID comes from the endpoint, not from the user's memory

The user types the model ID by hand today. The app already knows the provider's
`/v1/models` URL — `system_provider_connection.rs` carries one for every
built-in Provider, and `test_system_provider_connection` already calls it with
the user's key to verify it.

**Design.** A command returns the model IDs from that endpoint; the form offers
them as a list.

**The field stays free-text.** Two reasons it cannot become a closed picker:
relays commonly serve models absent from their own `/v1/models`, and the
series-ID feature depends on typing a value that is deliberately *not* a literal
model ID (`claude-sonnet-5` to cover `claude-sonnet-5-20260514`, via the prefix
match in `find_provider_model_pricing_row`).

**Redaction constraint — do not lose this.** The connection client currently
returns `response.status().as_u16()` and drops the body. That is deliberate:
`system_provider_connection.rs` has tests asserting the serialized result
contains neither the API key nor the upstream body. Reading the body to list
models must therefore parse it, keep only the model IDs, and discard the rest.
Passing the raw body outward would break a guarantee that is currently tested.

## Split

Backend (migration, nullable columns, per-field merge, model-list command) is
Codex's. The form is Claude's.
