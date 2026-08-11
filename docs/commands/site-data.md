# `ce site-data`

Generates the immutable public site-data JSON that the static library site
consumes. The output is written **atomically** to a caller-supplied directory:
readers (web CI, preview servers) see either the previous version or the new
version and never a partially-written tree.

## Subcommands

### `ce site-data generate --output <dir>`

Produces `<dir>/site-data.json` containing the projected `SiteData` DTO
defined by the `site-schema` crate. The DTO round-trips against
`web/schema/site-data-v1.schema.json`.

- `--output <dir>` — target directory. Missing parents are created.
- `--mode production|preview` — `production` (default) enforces a complete
  `[library.site]` block; `preview` allows uncommitted trees and missing
  optional metadata.

The command runs entirely offline: no Node, Astro, or Pagefind binary is
invoked. Downstream tools consume the JSON directly.

## Behaviour

- The projection is deterministic; two runs on the same repository state
  produce byte-identical output.
- Non-public libraries never appear in dependencies, reverse edges,
  relations, evidence links, or diagnostics.
- Locations survive only when the reference points at the target's own
  source file; non-entry solution diagnostics carry a `location_notice`.
- Symbol and diagnostic ordering follows spec §14: severity → location →
  code → message.

## Related files

- Schema: `web/schema/site-data-v1.schema.json`
- Projection: `crates/usecases/src/site_data.rs`
- Atomic write: `crates/infrastructure/src/repository_impl/site_data_repository_impl.rs`

## Status

Atomic-write repository and the pure projection are implemented and tested.
The end-to-end CLI wiring — analyzer dispatch, verification-record load, and
git-history probes — depends on plan 052 components; until those land, the
`ce site-data generate` subcommand is scaffolded but not connected.
