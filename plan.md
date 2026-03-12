# TUI Managed Proxy Dashboard Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn proxy from a developer-only foreground tool into a normal TUI feature: users can start it from TUI, see a clear dashboard on the main page, and leave it running after exiting TUI until they manually stop it.

**Architecture:** Keep the existing proxy server and manual takeover backend, but add a small managed-session lifecycle on top: TUI starts a background `cc-switch proxy serve` child process for the current app, persists its PID/session metadata, and can stop it later. Surface the runtime through richer status reads so the main page can render a visible proxy dashboard without exposing failover/breaker/queue controls.

**Tech Stack:** Rust, tokio, axum, reqwest, rusqlite, clap, ratatui, std::process.

---

## Product Guardrails

- **Do not implement automatic failover.**
- Do not auto-switch providers after errors.
- Keep provider routing pinned to the app's current provider at request time.
- TUI should expose one simple start/stop flow for the current app, not a full proxy control center.
- The dashboard should feel obvious and alive, but stay text-first and terminal-native rather than turning into a complex animation project.
- Exiting TUI must not stop a managed proxy session.
- Stopping the proxy manually must restore the current app from takeover.

## Working Assumption For This Plan

- “Start proxy” in TUI means: start a managed background proxy session **and** take over the current TUI app (`Claude` / `Codex` / `Gemini`) so traffic actually goes through cc-switch.
- “Stop proxy” in TUI means: restore takeover for that app and stop the managed session if it is no longer needed.

## Why This Plan Replaces The Previous One

- The previous plan completed the manual-takeover backend and a small CLI/TUI surface.
- It did **not** deliver the product UX you just asked for:
  - TUI-only start
  - home-page proxy dashboard
  - proxy stays alive after leaving TUI
- This plan keeps the existing manual-takeover work, then layers the missing product behavior on top.

## Execution Rule For This Plan

After each task:

1. Run the task-specific tests.
2. Run the regression command listed for that task.
3. Dispatch a subagent to compare the finished task against upstream and confirm the backend stayed conceptually aligned while the TUI remained intentionally smaller.
4. Only then move to the next task.

---

### Task 1: Add A Managed Background Proxy Session Lifecycle

**Why:**
Right now proxy runtime is only truly started by a foreground `proxy serve` process. That blocks the “normal user only uses TUI” path. We need a small managed-session layer so TUI can start a background child process, detect it reliably, fetch real runtime status, and stop it later.

**Files:**
- Modify: `src-tauri/src/services/proxy.rs`
- Modify: `src-tauri/src/cli/commands/proxy.rs`
- Modify: `src-tauri/src/cli/mod.rs`
- Modify: `src-tauri/src/database/dao/settings.rs`
- Test: `src-tauri/tests/proxy_service.rs`
- Test: `src-tauri/tests/proxy_takeover.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/services/proxy.rs`

**Step 1: Write the failing tests**

- Add a focused test that proves a persisted external proxy PID can be stopped through `ProxyService`, not only an in-process runtime.
- Add a focused test that proves `get_status()` prefers the live `/status` snapshot from the managed child session when it is reachable, instead of only returning marker-level uptime/address data.
- Add a focused test that proves stopping the managed proxy also restores manual takeover state for the current app.

**Step 2: Run tests to verify failure**

Run: `cargo test --test proxy_service -- --nocapture`

Expected: FAIL because external managed-session stop + rich status polling do not exist yet.

**Step 3: Write the minimal implementation**

- Extend `ProxyService` with a small managed-session lifecycle:
  - start a detached/background child process using the current binary and `proxy serve --takeover <app>`
  - persist enough session metadata to find it again
  - stop that external process later by PID
- Teach `get_status()` to try `http://<listen>/status` for a richer runtime snapshot when the persisted external session is alive.
- Keep the backend semantics close to upstream proxy runtime/status behavior, but do **not** add failover or a full daemon platform.
- If the external session is stopped manually or is stale, clear stale marker state.

**Step 4: Run tests to verify success**

Run: `cargo test --test proxy_service -- --nocapture`

Expected: PASS.

**Step 5: Run focused regression coverage**

Run: `cargo test --test proxy_takeover -- --nocapture && cargo test --test provider_commands -- --nocapture`

Expected: PASS.

**Step 6: Upstream consistency check**

Dispatch a subagent to compare the managed-session/runtime-status path against upstream proxy lifecycle code and confirm the backend shape is still basically aligned for this repo’s smaller product surface.

---

### Task 2: Make The Main Page Show A Real Proxy Dashboard

**Why:**
The proxy must feel “on” from the TUI homepage. Right now status is buried in a help overlay. We need a clear home-page card/banner that makes the runtime visible and gives the user immediate confidence that cc-switch is actively sitting in the traffic path.

**Files:**
- Modify: `src-tauri/src/cli/tui/data.rs`
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Modify: `src-tauri/src/cli/tui/app.rs`
- Modify: `src-tauri/src/cli/i18n.rs`
- Test: `src-tauri/src/cli/tui/app.rs`
- Test: `src-tauri/tests/proxy_service.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/commands/proxy.rs`

**Step 1: Write the failing tests**

- Add a TUI/home rendering test that proves the main page now shows a proxy card or banner when the managed proxy is off.
- Add a TUI/home rendering test that proves the running state shows current app, listen address, uptime, and current routed provider when the proxy is on.
- Add a focused test that proves the dashboard text explicitly communicates “manual routing through cc-switch” rather than automatic failover.

**Step 2: Run tests to verify failure**

Run: `cargo test proxy_ -- --nocapture`

Expected: FAIL because the main page has no proxy dashboard yet.

**Step 3: Write the minimal implementation**

- Extend `UiData::proxy` to carry the richer status fields needed by the homepage dashboard.
- Add a prominent proxy section to `Route::Main`:
  - off state: clear CTA / explanation
  - on state: running badge, current app takeover, listen address, uptime, request counters, routed provider, last error if any
- Keep the visuals terminal-native: bold title, status badge, compact metrics, obvious “cc-switch is in the middle” copy.
- Do not add a complicated multi-pane observability UI.

**Step 4: Run tests to verify success**

Run: `cargo test proxy_ -- --nocapture`

Expected: PASS.

**Step 5: Upstream consistency check**

Dispatch a subagent to confirm the new dashboard is smaller than upstream’s larger proxy/failover product surface, while still honestly representing the backend state.

---

### Task 3: Let Users Start/Stop Proxy From TUI Only

**Why:**
This is the actual product-flow task. A normal user should be able to enter TUI, press one obvious action to start proxy for the current app, see the dashboard light up, leave TUI, and come back later to stop it manually.

**Files:**
- Modify: `src-tauri/src/cli/tui/app.rs`
- Modify: `src-tauri/src/cli/tui/mod.rs`
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Modify: `src-tauri/src/cli/tui/data.rs`
- Modify: `src-tauri/src/cli/i18n.rs`
- Modify: `src-tauri/src/services/proxy.rs`
- Test: `src-tauri/src/cli/tui/app.rs`
- Test: `src-tauri/tests/proxy_service.rs`
- Test: `src-tauri/tests/proxy_takeover.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/services/proxy.rs`

**Step 1: Write the failing tests**

- Add a TUI action test that proves the main-page proxy CTA emits “start managed proxy for current app”.
- Add a TUI action test that proves the running dashboard emits “stop/restore current app” instead of sending the user to another terminal.
- Add a service/integration test that proves exiting and reloading app state still shows the managed proxy session as running.

**Step 2: Run tests to verify failure**

Run: `cargo test proxy_ -- --nocapture`

Expected: FAIL because the current TUI cannot start/stop proxy itself.

**Step 3: Write the minimal implementation**

- Add one simple home-page action for the current app:
  - when off: start managed session + take over current app
  - when on for current app: restore + stop managed session
- Refresh `UiData` after the action so the homepage dashboard updates immediately.
- Make sure the TUI quit path does **not** auto-stop the managed proxy.
- Keep the user surface intentionally tiny: one obvious action, one dashboard, one restore path.

**Step 4: Run tests to verify success**

Run: `cargo test proxy_ -- --nocapture`

Expected: PASS.

**Step 5: Run final focused verification**

Run: `cargo fmt && cargo test --test proxy_service -- --nocapture && cargo test --test proxy_takeover -- --nocapture && cargo test --test provider_commands -- --nocapture && cargo test proxy_ -- --nocapture`

Expected: PASS.

**Step 6: Upstream consistency check**

Dispatch a subagent to confirm that the backend remains close enough to upstream proxy behavior, while the TUI product flow intentionally stays simpler and does not expose failover/breaker/queue tooling.

---

## Explicitly Deferred

- Automatic failover
- Auto-switching providers after errors
- Queue routing / breaker editing / advanced failover tuning
- Tray integration
- A full daemon supervisor platform
- Multi-app proxy orchestration from the homepage beyond the current app
