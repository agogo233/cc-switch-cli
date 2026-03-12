# Proxy Phase C Manual Takeover Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move the local proxy from a Phase B foreground debug server to an upstream-shaped manual takeover runtime, without adding automatic failover.

**Architecture:** Keep the existing multi-app proxy routes and per-app timeout/retry behavior, but pull the runtime boundary closer to upstream by introducing `handler_context`, `provider_router`, and response pipeline helpers. Add per-app takeover and restore as explicit service actions only. TUI stays thin: show status, show takeover state, and expose only the minimum manual controls.

**Tech Stack:** Rust, tokio, axum, reqwest, rusqlite, clap, ratatui.

---

## Scope Guardrails

- Do not implement automatic failover.
- Do not route across provider queues.
- Do not add failover queue editing to CLI or TUI.
- Do not add tray or GUI-specific runtime behavior.
- Keep `auto_failover_enabled` as a stored compatibility field only.
- Prefer upstream naming and file boundaries when they fit the CLI repo.

## Runtime Model

- No daemon in this phase.
- Manual takeover is only valid while the local proxy server is running inside the current long-lived CLI or TUI process.
- `proxy serve` remains a foreground process. Any takeover-capable flow must either run inside that foreground process or start the same in-process server before rewriting live config.
- There are no one-shot CLI takeover or restore commands in this phase. CLI takeover is only exposed through long-running `proxy serve` session flags.
- On normal shutdown, the service restores all active takeovers before the process exits.
- On the next launch, startup recovery repairs stale localhost takeover state left behind by crashes or interrupted sessions.

## State Model

- Runtime truth comes from `ProxyService` and `ProxyStatus`: whether the proxy server is running, which app takeovers are active, and which listen address is currently valid.
- Persistent per-app truth comes from the existing per-app proxy config row. The app-level `enabled` flag is the source of truth for manual takeover intent in this phase.
- `proxy_enabled` stays as a thin global summary flag for simple UX and compatibility. It must not become an independent routing switch.
- `auto_failover_enabled` remains inert compatibility data.
- Any restore marker such as `live_takeover_active` is recovery metadata only. It does not participate in routing decisions.

## Checkpoint Strategy

- Checkpoint 0: commit the current Phase B proxy baseline before new behavior work starts.
- Checkpoint 1: align the runtime pipeline with upstream module boundaries, while keeping single-provider routing.
- Checkpoint 2: add manual per-app takeover and restore lifecycle, without failover.
- Checkpoint 3: expose the smallest useful CLI/TUI takeover controls.

## Checkpoint 0: Freeze the Current Phase B Baseline

**Files:**
- Modify: current proxy runtime and tests already in the worktree
- Add: `docs/plans/2026-03-10-proxy-phase-c-manual-takeover.md`

**Step 1: Verify the current baseline**
- Run `cargo fmt`.
- Run `cargo test --test proxy_claude_streaming -- --nocapture`.
- Run `cargo test --test proxy_multi_app_passthrough -- --nocapture`.
- Run `cargo test proxy_ -- --nocapture`.
- Run `cargo test provider_model_roundtrip_preserves_phase2_fields -- --nocapture`.

**Step 2: Commit the baseline**
```bash
git add docs/plans/2026-03-10-proxy-phase-c-manual-takeover.md src-tauri
git commit -m "feat(proxy): stabilize phase b multi-app runtime"
```

### Task 1: Align the Runtime Pipeline with Upstream Boundaries

**Files:**
- Modify: `src-tauri/src/proxy/mod.rs`
- Modify: `src-tauri/src/proxy/server.rs`
- Modify: `src-tauri/src/proxy/handlers.rs`
- Modify: `src-tauri/src/proxy/forwarder.rs`
- Modify: `src-tauri/src/proxy/response.rs`
- Create: `src-tauri/src/proxy/handler_context.rs`
- Create: `src-tauri/src/proxy/provider_router.rs`
- Create: `src-tauri/src/proxy/response_handler.rs`
- Test: `src-tauri/tests/proxy_claude_openai_chat.rs`
- Test: `src-tauri/tests/proxy_claude_streaming.rs`
- Test: `src-tauri/tests/proxy_multi_app_passthrough.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/proxy/handler_context.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/proxy/provider_router.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/proxy/response_handler.rs`

**Step 1: Write the failing tests**
- Add a focused test that proves streaming request accounting does not mark a request successful before the stream body finishes.
- Add a focused test that proves the router still uses only the current provider for each app, even if compatibility fields such as `auto_failover_enabled` are present.

**Step 2: Run the tests to verify failure**
Run: `cd src-tauri && cargo test proxy_ -- --nocapture`
Expected: FAIL because request lifecycle accounting and module boundaries are still handled directly inside `handlers.rs`.

**Step 3: Write the minimal implementation**
- Introduce `HandlerContext` to carry app type, provider, proxy config, and request metadata through the pipeline.
- Introduce `ProviderRouter` but keep its decision rule simple: resolve only the current provider for the app.
- Move response success/failure accounting out of the early handler path and closer to body completion handling.
- Extract only the response-layer logic that is already used today into `response_handler.rs` so `handlers.rs` becomes a thinner coordinator. Do not create extra modules unless the tests force the split.
- Keep existing timeout and retry semantics unchanged unless a failing test proves otherwise.

**Step 4: Run the tests to verify success**
Run: `cd src-tauri && cargo test proxy_ -- --nocapture`
Expected: PASS.

**Step 5: Commit**
```bash
git add src-tauri/src/proxy src-tauri/tests
git commit -m "refactor(proxy): align runtime pipeline with upstream"
```

### Task 2: Add Manual Per-App Takeover and Restore Lifecycle

**Files:**
- Modify: `src-tauri/src/services/proxy.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/database/dao/proxy.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/codex_config.rs`
- Modify: `src-tauri/src/gemini_config.rs`
- Modify: `src-tauri/src/cli/commands/provider.rs`
- Test: `src-tauri/tests/proxy_service.rs`
- Create: `src-tauri/tests/proxy_takeover.rs`
- Test: `src-tauri/tests/provider_commands.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/services/proxy.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/database/dao/proxy.rs`

**Step 1: Write the failing tests**
- Add a service test that enables takeover for one app, verifies the live config points to the local proxy, then restores the original config.
- Add a startup recovery test that proves an interrupted takeover can be restored on the next launch.
- Add a provider switch test that proves switching providers while takeover is active uses the proxy-aware path.

**Step 2: Run the tests to verify failure**
Run: `cd src-tauri && cargo test proxy_takeover -- --nocapture`
Expected: FAIL because manual takeover and restore entry points do not exist yet.

**Step 3: Write the minimal implementation**
- Add explicit per-app takeover APIs to `ProxyService`, matching upstream naming where practical.
- Persist takeover state and backup metadata in the existing proxy DAO structures instead of inventing a new store.
- Update the live config writers for Claude, Codex, and Gemini so takeover can switch to the local proxy and restore safely.
- Add startup restore logic that repairs broken or stale takeover state, but do not start any automatic failover behavior.
- Make provider switching respect takeover state as a service-level invariant instead of a TUI-only special case.

**Step 4: Run the tests to verify success**
Run: `cd src-tauri && cargo test proxy_takeover -- --nocapture`
Expected: PASS.

**Step 5: Run regression coverage**
Run: `cd src-tauri && cargo test proxy_service -- --nocapture && cargo test proxy_takeover -- --nocapture && cargo test provider_commands -- --nocapture && cargo test proxy_ -- --nocapture`
Expected: PASS.

**Step 6: Commit**
```bash
git add src-tauri/src/services/proxy.rs src-tauri/src/database/dao/proxy.rs src-tauri/src/config.rs src-tauri/src/codex_config.rs src-tauri/src/gemini_config.rs src-tauri/src/cli/commands/provider.rs src-tauri/src/lib.rs src-tauri/src/main.rs src-tauri/tests
git commit -m "feat(proxy): add manual per-app takeover lifecycle"
```

### Task 3: Expose Minimal CLI/TUI Takeover Controls

**Files:**
- Modify: `src-tauri/src/cli/commands/proxy.rs`
- Modify: `src-tauri/src/cli/commands/mod.rs`
- Modify: `src-tauri/src/cli/mod.rs`
- Modify: `src-tauri/src/cli/i18n.rs`
- Modify: `src-tauri/src/cli/tui/data.rs`
- Modify: `src-tauri/src/cli/tui/app.rs`
- Modify: `src-tauri/src/cli/tui/mod.rs`
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Test: `src-tauri/tests/proxy_service.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/commands/proxy.rs`

**Step 1: Write the failing tests**
- Add a CLI parsing test for `proxy serve` takeover flags that keep the process in the foreground session.
- Add a TUI action test that toggles takeover for the selected app and refreshes the visible proxy snapshot.

**Step 2: Run the tests to verify failure**
Run: `cd src-tauri && cargo test proxy_ -- --nocapture`
Expected: FAIL because the manual takeover controls are not wired into CLI/TUI yet.

**Step 3: Write the minimal implementation**
- Keep CLI takeover attached to `proxy serve` so the process lifetime and takeover lifetime stay identical.
- Keep simple CLI read-only inspection such as `proxy show`, but do not introduce detached takeover commands.
- Keep TUI limited to status display, a takeover toggle per app, and short help text.
- Do not add queue editing, breaker panels, or failover controls.

**Step 4: Run the tests to verify success**
Run: `cd src-tauri && cargo test proxy_ -- --nocapture`
Expected: PASS.

**Step 5: Final verification**
Run: `cd src-tauri && cargo fmt && cargo test -- --nocapture`
Expected: PASS.

**Step 6: Commit**
```bash
git add src-tauri/src/cli src-tauri/tests
git commit -m "feat(tui): add minimal manual proxy takeover controls"
```

## Deferred After This Plan

- Automatic failover and failover switch management
- Failover queue editing and queue-aware provider routing
- Usage dashboards, request log UI, and pricing UI
- Tray integration or GUI event synchronization
- TUI editing for low-level timeout or retry policy values
