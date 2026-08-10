# Rust Library Symbol Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract stable searchable Rust symbols and byte-accurate source locations without changing dependency semantics.

**Architecture:** Extend the Rust syntax visitor with a symbol projection. The adapter reports language-neutral symbol kinds, qualified/search names, visibility, and one-based line/column locations; the core remains unaware of Rust traits or impl syntax.

**Tech Stack:** Rust 1.92.0, syn full/visit, proc-macro2 span-locations.

## Constraints

- **Branch:** `feat/044-library-rust-symbols`
- **Depends on:** plan 043 merged to `main`.
- Read specification sections 6.3, 6.6, 12.5, and 13.1.
- Do not create Rust-specific Web components or public DTO fields.
- Anonymous impl blocks are containers, not fabricated named symbols.
- Locationless symbols use the page fragment `#symbols`; located symbols later use generated line anchors.

### Task 1: Extract declarations and qualified names

**Files:**
- Create: `tools/library-analyzers/rust/src/symbols.rs`
- Modify: `tools/library-analyzers/rust/src/main.rs`
- Create: `tools/library-analyzers/rust/tests/symbols.rs`
- Create: `tools/library-analyzers/protocol/fixtures/rust-symbols-request.json`
- Create: `tools/library-analyzers/protocol/fixtures/rust-symbols-response.json`

**Interfaces:**

```rust
pub fn analyze_symbols(
    source: &str,
    target_path: &str,
    module_path: &[String],
) -> SymbolAnalysis;
```

- [ ] Write failing fixtures for modules, structs, enums/variants, traits, trait methods, impl methods,
      functions, type aliases, constants/statics, macros, nested declarations, and Unicode before spans.
- [ ] Run `cargo test -p ce-library-rust-analyzer symbols`; observe missing symbol analysis.
- [ ] Implement stable source-order traversal and language-neutral kinds such as `trait`, `method`, and `type`.
- [ ] Emit qualified names from lexical/module ownership and deduplicated exact search names.
- [ ] Invoke `/commit` with `feat: extract Rust library symbols`.

### Task 2: Validate locations and partial states

**Files:**
- Modify: `tools/library-analyzers/rust/src/symbols.rs`
- Modify: `tools/library-analyzers/rust/tests/symbols.rs`
- Modify: `crates/usecases/tests/library_analysis.rs`

- [ ] Add failing cases for one-based bounds, CRLF input, duplicate names, generated/no span, malformed syntax,
      public/private visibility, and symbols outside the requested target.
- [ ] Reject invalid spans before serialization; retain valid symbols and mark `partial` for recoverable syntax gaps.
- [ ] Prove a symbol-only failure leaves dependency analysis complete and does not block verification fingerprinting.
- [ ] Run `cargo test -p ce-library-rust-analyzer && cargo test -p usecases library_analysis`.
- [ ] Invoke `/commit` with `test: lock Rust symbol location behavior`.

### Task 3: Deliver Rust symbols

- [ ] Run handshake, Rust dependency fixture, Rust symbol fixture, and normalized snapshot tests.
- [ ] Run rollout repository verification and `git diff --check`.
- [ ] Invoke `/commit` with `docs: record Rust symbol adapter completion`.
- [ ] Invoke `/pr --base main`; link plan 044 and state that it unblocks plan 045.
- [ ] Invoke `/pr-review` to no new comments, wait for CI, and merge to `main`.
