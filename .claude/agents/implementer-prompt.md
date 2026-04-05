# Implementer Subagent Prompt Template

Use this template when dispatching an implementer subagent (GREEN phase).

```
Task tool (general-purpose):
  description: "Implement: [task name]"
  prompt: |
    You are implementing: [task name]

    ## Task Description

    [FULL TEXT of the checklist item or task — paste it here]

    ## Failing Tests to Make Pass

    [List the test names and file paths from the test-writer agent's report]

    ## Architecture

    4-layer DDD (Clean Architecture):
    - domain/     — entities, value objects (no dependencies)
    - usecases/   — port traits + services (depends on domain only)
    - interfaces/ — Controller, Input traits (depends on usecases)
    - infrastructure/ — HTTP, filesystem, clap (depends on interfaces)

    Dependency direction: infrastructure → interfaces → usecases → domain
    Inner layers must NOT import outer layers.

    Error handling: use `anyhow` + `thiserror`. Do NOT use `E: Error + 'static` type params.

    ## Your Job

    Write the minimal implementation to make the failing tests pass.

    Steps:
    1. Read the failing tests to understand the required behavior
    2. Write the minimal code needed — no extra features, no speculative abstractions
    3. Run `cargo test [test_name] 2>&1` to confirm the target tests now PASS
    4. Run `cargo test 2>&1` to confirm ALL tests still pass
    5. Fix any regressions before reporting back

    ## Rules

    - Write only what is needed to pass the tests
    - Do NOT modify existing tests
    - YAGNI: do not add features not required by the tests
    - Follow existing code patterns in the crate
    - Output and comments must be in English
    - Keep inner layers free of outer-layer imports

    ## Self-Review Before Reporting

    - [ ] Target tests pass
    - [ ] All other tests still pass (`cargo test` clean)
    - [ ] No implementation code written before seeing a failing test
    - [ ] No over-engineering beyond what tests require

    ## Report Format

    When done, report:
    - **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED
    - What you implemented (files changed)
    - `cargo test` output (copy the actual output showing all tests pass)
    - Any concerns or caveats
```
