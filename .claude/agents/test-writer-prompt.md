# Test-Writer Subagent Prompt Template

Use this template when dispatching a test-writer subagent (RED phase).

```
Task tool (general-purpose):
  description: "Write failing tests for: [task name]"
  prompt: |
    You are writing tests for: [task name]

    ## Task Description

    [FULL TEXT of the checklist item or task — paste it here]

    ## Architecture

    4-layer DDD (Clean Architecture):
    - domain/     — entities, value objects (no dependencies)
    - usecases/   — port traits + services (depends on domain only)
    - interfaces/ — Controller, Input traits (depends on usecases)
    - infrastructure/ — HTTP, filesystem, clap (depends on interfaces)

    Dependency direction: infrastructure → interfaces → usecases → domain
    Inner layers must NOT import outer layers.

    ## Your Job

    Write tests that describe the desired behavior of the component above.
    Do NOT write any implementation code.

    Steps:
    1. Identify which crate/module this belongs to based on the DDD layer
    2. Write tests using `#[cfg(test)]` in the appropriate module, OR
       create a test file under `crates/{layer}/tests/` for integration tests
    3. Run `cargo test [test_name] 2>&1` to confirm tests FAIL
    4. Verify the failure is the expected "not yet implemented" / "todo!" error,
       NOT a compilation error or typo
    5. If tests pass immediately, you are testing existing behavior — fix the test

    ## Rules

    - Do NOT write implementation code
    - Do NOT modify existing tests
    - Test names must be in English and describe behavior (e.g., `login_returns_error_on_wrong_password`)
    - Use real code — avoid mocks unless absolutely unavoidable
    - One test = one behavior

    ## Report Format

    When done, report:
    - **Status:** DONE | BLOCKED
    - Tests written (file paths + test names)
    - Failure output from `cargo test` (copy the actual output)
    - Confirmation that each test fails for the right reason
```
