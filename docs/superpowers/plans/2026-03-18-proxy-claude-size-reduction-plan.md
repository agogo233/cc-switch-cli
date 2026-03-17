# Proxy/Claude File Size Reduction Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce the `proxy/Claude` hotspot files to the agreed size thresholds without changing runtime behavior, with first priority on eliminating `>1600` line files.

**Architecture:** Keep public entrypoints and runtime behavior intact. Split only along low-risk boundaries first: test fixtures versus test scenarios, `#[cfg(test)]` modules versus production code, and pure helper logic versus orchestration code. Treat `forwarder.rs` as a thin façade over extracted helper modules, but do not rewrite its failover/timeout orchestration in this round.

**Tech Stack:** Rust, Cargo integration tests, Rust unit tests, Axum test upstream handlers, existing Claude proxy regression suites

**Hard size rule:** The same `<=1600` hard stop applies to new files created by this plan. Do not solve one oversized file by creating another oversized child module.

---

## Chunk 1: Claude Integration Test Decomposition

### Task 1: Split `proxy_claude_streaming` into helper and scenario modules

**Files:**
- Modify: `src-tauri/tests/proxy_claude_streaming.rs`
- Create: `src-tauri/tests/proxy_claude_streaming/helpers.rs`
- Create: `src-tauri/tests/proxy_claude_streaming/success_cases.rs`
- Create: `src-tauri/tests/proxy_claude_streaming/error_cases.rs`
- Create: `src-tauri/tests/proxy_claude_streaming/logging_cases.rs`

- [ ] **Step 1: Record the baseline before moving code**

Run:

```bash
cargo test --test proxy_claude_streaming && wc -l src-tauri/tests/proxy_claude_streaming.rs
```

Expected: test target passes, and the line count is well above the target threshold.

- [ ] **Step 2: Turn the top-level test crate into a thin module entrypoint**

Keep `src-tauri/tests/proxy_claude_streaming.rs` as the integration-test crate root, but reduce it to:

- shared imports needed across modules
- `mod helpers;`
- `mod success_cases;`
- `mod error_cases;`
- `mod logging_cases;`

Do not rename the test target. Cargo should still run it as `proxy_claude_streaming`.

- [ ] **Step 3: Move only fixtures and helper utilities first**

Move these kinds of items into `src-tauri/tests/proxy_claude_streaming/helpers.rs`:

- fake upstream state structs
- helper Axum handlers that are reused by multiple tests
- request/log polling helpers
- shared request builders and fixture setup helpers

Do not change assertions or scenario names in this step.

- [ ] **Step 4: Move scenario tests by behavior group**

Split the existing tests into the three scenario modules:

- `success_cases.rs` for happy-path streaming and buffered-success fallback
- `error_cases.rs` for timeout, malformed upstream body, passthrough fallback, and transformed error cases
- `logging_cases.rs` for request logging, usage logging, session-id, and error-summary related cases

Keep test names unchanged unless the current name becomes impossible after the move.

- [ ] **Step 5: Re-run the focused test target**

Run:

```bash
cargo test --test proxy_claude_streaming
```

Expected: PASS with the same test count or a larger test count only if a split exposed previously inlined tests.

- [ ] **Step 6: Verify the file-size goal for this file**

Run:

```bash
wc -l src-tauri/tests/proxy_claude_streaming.rs src-tauri/tests/proxy_claude_streaming/*.rs
```

Expected: `src-tauri/tests/proxy_claude_streaming.rs` is now `<=1600`, and ideally much smaller because helpers and grouped scenarios moved out.

Also verify every newly created file under `src-tauri/tests/proxy_claude_streaming/` is `<=1600`.

- [ ] **Step 7: Do not commit**

Per repo instructions, do not create a git commit unless the user explicitly asks.

### Task 2: Split `proxy_claude_openai_chat` into helper and scenario modules

**Files:**
- Modify: `src-tauri/tests/proxy_claude_openai_chat.rs`
- Create: `src-tauri/tests/proxy_claude_openai_chat/helpers.rs`
- Create: `src-tauri/tests/proxy_claude_openai_chat/transform_cases.rs`
- Create: `src-tauri/tests/proxy_claude_openai_chat/error_cases.rs`
- Create: `src-tauri/tests/proxy_claude_openai_chat/logging_cases.rs`

- [ ] **Step 1: Capture the baseline and guardrail**

Run:

```bash
cargo test --test proxy_claude_openai_chat && wc -l src-tauri/tests/proxy_claude_openai_chat.rs
```

Expected: PASS, and the file is above the desired `1200` threshold.

- [ ] **Step 2: Convert the top-level file into a crate root with submodules**

Keep `src-tauri/tests/proxy_claude_openai_chat.rs` as the integration-test crate entrypoint, but move reusable helpers and grouped scenarios out into sibling modules.

- [ ] **Step 3: Move fixtures and upstream handler helpers into `helpers.rs`**

Extract:

- shared upstream-state structs
- common request/response helper functions
- repeated Axum route handlers
- repeated DB/request-log parsing helpers

Do not change scenario behavior.

- [ ] **Step 4: Group tests by behavior**

Move tests into:

- `transform_cases.rs` for request/response shape mapping and success-path transformations
- `error_cases.rs` for non-success passthrough, malformed upstream, transform failures, and status preservation
- `logging_cases.rs` for request-log, usage-log, session-id, and cost/pricing assertions

- [ ] **Step 5: Re-run the focused test target**

Run:

```bash
cargo test --test proxy_claude_openai_chat
```

Expected: PASS.

- [ ] **Step 6: Verify the size target**

Run:

```bash
wc -l src-tauri/tests/proxy_claude_openai_chat.rs src-tauri/tests/proxy_claude_openai_chat/*.rs
```

Expected: `src-tauri/tests/proxy_claude_openai_chat.rs` is `<=1200`.

Also verify every newly created file under `src-tauri/tests/proxy_claude_openai_chat/` is `<=1600`.

- [ ] **Step 7: Do not commit**

Per repo instructions, do not create a git commit unless the user explicitly asks.

## Chunk 2: Low-Risk Runtime File Slimming

### Task 3: Move `response.rs` helpers and tests behind a small module boundary

**Files:**
- Modify: `src-tauri/src/proxy/response.rs`
- Create: `src-tauri/src/proxy/response/error_summary.rs`
- Create: `src-tauri/src/proxy/response/tests.rs`

- [ ] **Step 1: Lock the current behavior with focused unit tests**

Run:

```bash
cargo test proxy::response::tests
```

Expected: PASS before any movement.

- [ ] **Step 2: Extract pure summary helpers first**

Move only pure, low-risk helpers into `src-tauri/src/proxy/response/error_summary.rs`, such as:

- upstream body summarization
- JSON error-message extraction
- text truncation helpers

Keep `PreparedResponse`, public builder entrypoints, and timeout read orchestration in `response.rs` for this round.

- [ ] **Step 3: Move the `#[cfg(test)]` block into `response/tests.rs`**

Re-export or `mod` the test module from `response.rs`, but keep production behavior untouched.

- [ ] **Step 4: Re-run focused unit tests and Claude response regressions**

Run:

```bash
cargo test --no-run && cargo test proxy::response::tests && cargo test --test proxy_claude_openai_chat && cargo test --test proxy_claude_streaming
```

Expected: PASS.

- [ ] **Step 5: Verify that this file did not expand scope**

Do a quick diff review and confirm the change is limited to helper/test extraction. If any cross-provider behavior changed, stop and revisit the design before continuing.

### Task 4: Move `response_handler.rs` tests and optional recorder internals only if the boundary stays clean

**Files:**
- Modify: `src-tauri/src/proxy/response_handler.rs`
- Create: `src-tauri/src/proxy/response_handler/tests.rs`
- Optional Create: `src-tauri/src/proxy/response_handler/stream_recorder.rs`

- [ ] **Step 1: Capture the current focused baseline**

Run:

```bash
cargo test proxy::response_handler::tests
```

Expected: PASS if the test module already compiles under that path. If not, use the nearest focused test filter that exercises the current unit tests.

- [ ] **Step 2: Move the `#[cfg(test)]` module out of the main file**

Keep production entrypoints in place and move only unit-test code into `src-tauri/src/proxy/response_handler/tests.rs`.

- [ ] **Step 3: Extract recorder internals only if the seam is obvious**

If `StreamingOutcomeRecorder` can move into `stream_recorder.rs` without changing signatures or visibility in surprising ways, do it. If the move starts pulling in unrelated handler coordination logic, skip this extraction in this round.

- [ ] **Step 4: Re-run focused tests plus the Claude regression suites most sensitive to this file**

Run:

```bash
cargo test --no-run && cargo test --test proxy_claude_openai_chat && cargo test --test proxy_claude_streaming && cargo test --test proxy_upstream_error_summary
```

Expected: PASS.

- [ ] **Step 5: Verify that behavior is still limited to module movement**

Confirm no changes to:

- request logging timing
- success/failure accounting
- buffered-success fallback handling
- upstream error summary propagation

### Task 5: Extract `provider_router.rs` tests and endpoint rewrite only if it stays behavior-neutral

**Files:**
- Modify: `src-tauri/src/proxy/provider_router.rs`
- Optional Create: `src-tauri/src/proxy/provider_router/upstream_endpoint.rs`
- Create: `src-tauri/src/proxy/provider_router/tests.rs`

- [ ] **Step 1: Baseline the current focused coverage**

Run the narrowest available provider-router unit tests. If no stable narrow filter exists, use:

```bash
cargo test --test proxy_claude_forwarder_alignment
```

Expected: PASS before movement.

- [ ] **Step 2: Move the `#[cfg(test)]` block out first**

Perform only the test-module extraction if it is clean.

- [ ] **Step 3: Extract endpoint rewrite helpers only if they remain pure**

Move upstream-endpoint rewrite logic into `upstream_endpoint.rs` only if it is purely transformational. Do not move provider selection, breaker registry, or current-provider synchronization in this round.

- [ ] **Step 4: Re-run focused verification**

Run:

```bash
cargo test --no-run && cargo test --test proxy_claude_openai_chat && cargo test --test proxy_claude_streaming && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_model_mapping
```

Expected: PASS.

- [ ] **Step 5: Skip rather than force the extraction**

If the endpoint boundary is not obviously pure, keep the production code where it is and treat this task as “tests extracted only.”

## Chunk 3: `forwarder.rs` Size Reduction Without Orchestration Rewrite

### Task 6: Move `forwarder.rs` tests out of the runtime file

**Files:**
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Create: `src-tauri/src/proxy/forwarder/tests/mod.rs`
- Create: `src-tauri/src/proxy/forwarder/tests/provider_failover.rs`
- Create: `src-tauri/src/proxy/forwarder/tests/error_paths.rs`
- Optional Create: `src-tauri/src/proxy/forwarder/tests/request_building.rs`

- [ ] **Step 1: Record the current size and regression baseline**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_response_parity
```

Expected: `forwarder.rs` is still well above the target, and the regression tests pass.

- [ ] **Step 2: Move the `#[cfg(test)]` block into a small test-module tree, not one giant file**

Split the existing unit tests into `forwarder/tests/mod.rs` plus behavior-grouped submodules. Do not create a new single file that is itself `>1600` lines.

Do not change production logic in this step. The goal is to strip test bulk from the runtime file first without creating a second oversized file.

- [ ] **Step 3: Re-run the focused forwarder regressions**

Run:

```bash
cargo test --no-run && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_response_parity && cargo test --test proxy_claude_streaming
```

Expected: PASS.

- [ ] **Step 4: Measure the remaining gap**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/forwarder/tests/*.rs src-tauri/src/proxy/forwarder/tests/mod.rs
```

Expected: `forwarder.rs` is materially smaller, and every created test file is `<=1600`.

### Task 7: Extract request-building helpers into their own module

**Files:**
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Create: `src-tauri/src/proxy/forwarder/request_builder.rs`

- [ ] **Step 1: Move request-construction code into `request_builder.rs`**

Extract only code that:

- maps body/headers into upstream request data
- applies deterministic request-shape tweaks
- does not own retry/failover orchestration

Keep public APIs stable from the point of view of `forwarder.rs`.

- [ ] **Step 2: Re-run the focused regression set**

Run:

```bash
cargo test --no-run && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_response_parity && cargo test --test proxy_claude_streaming && cargo test --test proxy_claude_openai_chat
```

Expected: PASS.

- [ ] **Step 3: Check the size delta before moving on**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/forwarder/request_builder.rs
```

Expected: `forwarder.rs` is smaller, and `request_builder.rs` is `<=1600`.

### Task 8: Extract error-classification helpers into their own module

**Files:**
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Create: `src-tauri/src/proxy/forwarder/error_classification.rs`

- [ ] **Step 1: Move error/result classification helpers into `error_classification.rs`**

Extract only code that:

- normalizes upstream failure categories
- maps timeout/read errors into existing `ProxyError` values
- stays pure or near-pure

Do not rewrite the retry loop or the permit-lifecycle flow in this step.

- [ ] **Step 2: Re-run the full forwarder-sensitive regression set**

Run:

```bash
cargo test --no-run && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_response_parity && cargo test --test proxy_claude_model_mapping && cargo test --test proxy_upstream_error_summary && cargo test --test proxy_claude_streaming && cargo test --test proxy_claude_openai_chat
```

Expected: PASS.

- [ ] **Step 3: Check the hard size target after the second extraction**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/forwarder/request_builder.rs src-tauri/src/proxy/forwarder/error_classification.rs
```

Expected: `src-tauri/src/proxy/forwarder.rs` moves materially closer to `<=1600`, and every created helper file is `<=1600`.

### Task 9: If `forwarder.rs` is still above `1600`, perform one final bounded extraction and do not mark the plan complete until it passes the hard stop

**Files:**
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Optional Create: `src-tauri/src/proxy/forwarder/send.rs`
- Optional Create: one additional helper module with a name that matches the extracted pure boundary

- [ ] **Step 1: Re-check the measured line count before touching more code**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/forwarder/*.rs
```

Expected: if `forwarder.rs <= 1600`, skip this task. If it is still above `1600`, this task is mandatory.

- [ ] **Step 2: Extract exactly one more bounded helper seam**

Allowed:

- one more pure helper extraction
- one thin shared send helper used without changing orchestration order

Not allowed:

- failover loop rewrite
- permit lifecycle rewrite
- timeout policy rewrite

If the only apparent way to cross the threshold is to rewrite orchestration logic, stop the implementation as `BLOCKED`, surface that to the user, and do not claim the plan is complete.

- [ ] **Step 3: Re-run the full forwarder-sensitive regression set again**

Run:

```bash
cargo test --no-run && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_response_parity && cargo test --test proxy_claude_model_mapping && cargo test --test proxy_upstream_error_summary && cargo test --test proxy_claude_streaming && cargo test --test proxy_claude_openai_chat
```

Expected: PASS.

- [ ] **Step 4: Re-run the size audit and enforce the hard stop**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/forwarder/*.rs
```

Expected:

- `src-tauri/src/proxy/forwarder.rs <= 1600`
- every created file under `src-tauri/src/proxy/forwarder/` is also `<=1600`

If these conditions are not met, the implementation is not done.

## Chunk 4: Final Verification And Audit

### Task 10: Run the full agreed verification set and record the resulting file sizes

**Files:**
- Verify: `src-tauri/src/proxy/forwarder.rs`
- Verify: `src-tauri/tests/proxy_claude_streaming.rs`
- Verify: `src-tauri/tests/proxy_claude_openai_chat.rs`
- Verify: `src-tauri/src/proxy/response.rs`
- Verify: `src-tauri/src/proxy/response_handler.rs`
- Verify: `src-tauri/src/proxy/provider_router.rs`

- [ ] **Step 1: Run the compile gate**

Run:

```bash
cargo test --no-run
```

Expected: PASS.

- [ ] **Step 2: Run the final focused regression suite**

Run:

```bash
cargo test proxy::response::tests && cargo test proxy::usage::parser::tests && cargo test --test proxy_claude_openai_chat && cargo test --test proxy_claude_streaming && cargo test --test proxy_claude_forwarder_alignment && cargo test --test proxy_claude_response_parity && cargo test --test proxy_claude_model_mapping && cargo test --test proxy_upstream_error_summary
```

Expected: PASS.

- [ ] **Step 3: Record the final size audit**

Run:

```bash
wc -l src-tauri/src/proxy/forwarder.rs src-tauri/src/proxy/forwarder/*.rs src-tauri/src/proxy/response.rs src-tauri/src/proxy/response/*.rs src-tauri/src/proxy/response_handler.rs src-tauri/src/proxy/response_handler/*.rs src-tauri/src/proxy/provider_router.rs src-tauri/src/proxy/provider_router/*.rs src-tauri/tests/proxy_claude_streaming.rs src-tauri/tests/proxy_claude_streaming/*.rs src-tauri/tests/proxy_claude_openai_chat.rs src-tauri/tests/proxy_claude_openai_chat/*.rs
```

Expected:

- `src-tauri/src/proxy/forwarder.rs <= 1600`
- `src-tauri/tests/proxy_claude_streaming.rs <= 1600`
- `src-tauri/tests/proxy_claude_openai_chat.rs <= 1200`
- every newly created file covered by this plan is `<=1600`

For the other files, verify they did not grow and that their boundaries are clearer than before.

- [ ] **Step 4: Write down any files that still need a second round**

If some files remain above `1200` but under the hard stop and the only next moves are higher-risk orchestration splits, document them as follow-up work rather than expanding scope.

- [ ] **Step 5: Do not commit unless the user explicitly asks**

This refactor should stay uncommitted until the user asks for a commit.
