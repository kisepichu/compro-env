# Copilot Instructions

## Language

Think and reason in English. Write all reviews, suggestions, and explanations in Japanese.

## Spec-first development

Every feature has a spec in `docs/commands/<command>.md` (per-command) or `docs/spec.md` (overall).
Read the spec before touching implementation. When in doubt, spec wins over code.

## Verify spec ↔ implementation alignment

When reviewing or editing code, check that:

- Everything described in the spec is implemented (no missing behavior)
- Everything implemented matches the spec (no undocumented behavior)
- Domain entities in `crates/domain/src/entity.rs` match the field definitions in the spec
- Tera context variables exposed to templates match the table in `docs/commands/init.md`
- Test cases cover the examples given in the spec

If a gap is found, flag it clearly: fix the implementation **or** update the spec — not both without intent.

## Architecture (4-layer DDD)

```
domain/         entities & value objects — no external deps
usecases/       ports (traits) + services — depends on domain only
interfaces/     controllers & input traits — depends on usecases
infrastructure/ HTTP, filesystem, clap — depends on all layers
```

Dependency rule: inner layers must not import outer layers.
Error handling: `anyhow` + `thiserror`. Do not use `E: Error + 'static` type parameters.
