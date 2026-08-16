# `ce site-data`

Generates the immutable public site-data JSON that the static library site
consumes. The output is written to a caller-supplied directory in a way
that keeps partial trees out of the target path.

**Atomicity guarantee:** on Linux ≥ 3.15 with a filesystem that supports
`renameat2(RENAME_EXCHANGE)` (ext4 / xfs / btrfs with default mount
options), readers always observe either the old or the new tree — the
swap is a single kernel call. On non-Linux hosts, or on filesystems that
reject `RENAME_EXCHANGE` (some FUSE mounts, older ext4), the write falls
back to a rename-aside-then-rename-into sequence, which leaves a brief
window where the target directory is missing. See the module docstring
on `SiteDataRepositoryImpl` for details.

## Subcommands

### `ce site-data generate --output <dir>`

Produces `<dir>/site-data.json` containing the projected `SiteData` DTO
defined by the `site-schema` crate. The DTO round-trips against
`web/schema/site-data-v1.schema.json`.

- `--output <dir>` — target directory. Missing parents are created.
- `--mode production|preview` — `production` (default) requires a fully
  populated `[library.site]` block **and** a clean working tree; `preview`
  allows the entire `[library.site]` block to be omitted (individual
  fields are still all-or-nothing) and does not gate on uncommitted
  changes.

The command runs entirely offline: no Node, Astro, or Pagefind binary is
invoked. Downstream tools consume the JSON directly.

## Behaviour

- The projection function is deterministic: given identical inputs it
  produces byte-identical `SiteData`. The CLI's `site-data.json` will
  differ across runs by the `build.generated_at` timestamp (`Utc::now()`
  is captured on every run); every other field is stable when the
  repository state has not changed.
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

End-to-end wiring is in place: `ce site-data generate` invokes discovery,
runs the per-language analyzer via [`LibraryAdapterRunner`], loads
verification records from the on-disk repository, queries `git log` for
per-file `updated_at`, projects into [`SiteData`], and writes atomically.

Per-solution *current* fingerprints are recomputed inline: the generator
reuses `verification_closure` + `calculate_fingerprint` from
`crates/usecases/src/verification/fingerprint.rs` so a `Completed` record
whose hashed inputs still match the working tree surfaces as `Verified`,
while any source, closure library, adapter, or `[verify]`-block drift folds
the solution to `Stale` per spec §11.

Preprocess hooks are not invoked during recomputation — site-data is
offline — so records persisted with a source-mutating `[submit].preprocess`
hook can legitimately read as `Stale` until the source is re-verified.
