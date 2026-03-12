# App-Specific Managed Proxy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make proxy feel app-specific in TUI: `Claude`, `Codex`, and `Gemini` each get their own on/off state and current-app control, while `OpenCode` stays out if upstream does not support proxy.

**Architecture:** Keep one shared local proxy runtime, not three separate daemons. App independence lives in per-app takeover state and TUI behavior: starting proxy for the current app should either boot the managed runtime or attach the current app to an already-running managed runtime, and stopping should only tear the runtime down when no supported app is still attached. This stays close to upstream, avoids extra ports/processes, and matches the “single current-app card” UX.

**Tech Stack:** Rust, tokio, clap, ratatui, rusqlite, std::process.

---

## Product Guardrails

- Keep the homepage as one current-app proxy card, not a three-panel control center.
- `P` always means “toggle proxy for the current app”.
- `Claude`, `Codex`, and `Gemini` support this flow.
- `OpenCode` does not show proxy controls if upstream still treats it as unsupported.
- Do not introduce per-app proxy ports or multiple long-lived proxy daemons.
- Do not add automatic failover, queue controls, or breaker tuning.

## Current Findings

- Upstream still treats `OpenCode` proxy as unsupported in `/.upstream/cc-switch/src-tauri/src/services/proxy.rs:213` and `/.upstream/cc-switch/src-tauri/src/services/proxy.rs:376`.
- Our current backend already stores takeover state per app, but `set_managed_session_for_app()` still behaves like a single-app start/stop flow.
- The main TUI bug is semantic: if a managed runtime is already up for another app, the current app cannot cleanly “join” it from the homepage.

## Working Model For This Plan

- There is one shared managed proxy runtime.
- Each supported app has its own takeover flag and its own homepage state.
- Turning proxy on for the current app:
  - starts the managed runtime if it is not running
  - otherwise reuses the managed runtime and only enables takeover for the current app
- Turning proxy off for the current app:
  - restores that app from takeover
  - stops the managed runtime only if no supported app is still attached

---

### Task 1: Let A Second App Reuse The Existing Managed Runtime

**Files:**
- Modify: `src-tauri/src/services/proxy.rs`
- Test: `src-tauri/tests/proxy_service.rs`
- Test: `src-tauri/tests/proxy_takeover.rs`
- Check: `/.upstream/cc-switch/src-tauri/src/services/proxy.rs`

**Step 1: Write the failing test**

- Add a focused service test that starts a managed runtime for one supported app, then enables takeover for a second supported app without spawning a second runtime.
- Add a focused service test that proves disabling one app leaves the managed runtime alive while another supported app is still attached.
- Add a focused service test that proves `OpenCode` still returns an unsupported error instead of silently creating proxy state.

**Step 2: Run test to verify it fails**

Run: `cargo test --test proxy_service managed -- --nocapture`

Expected: FAIL because enabling a second app currently goes through `start_managed_session()` and errors when a runtime is already up.

**Step 3: Write minimal implementation**

- Update `set_managed_session_for_app()` in `src-tauri/src/services/proxy.rs`.
- For `enabled = true`:
  - keep `OpenCode` and other unsupported apps rejected through `takeover_app_from_str()`
  - if no runtime is running, keep the existing `start_managed_session()` path
  - if a managed external runtime is already running, call `enable_takeover_for_app()` instead of erroring
  - if a non-managed foreground runtime is running, return an explicit error instead of trying to piggyback on it
- For `enabled = false`:
  - keep `disable_takeover_for_app()`
  - preserve the existing “stop server only when last app is off” behavior

**Step 4: Run test to verify it passes**

Run: `cargo test --test proxy_service managed -- --nocapture`

Expected: PASS.

**Step 5: Run regression tests**

Run: `cargo test --test proxy_takeover -- --nocapture`

Expected: PASS.

---

### Task 2: Make TUI Think In Current-App Proxy State, Not Global Runtime State

**Files:**
- Modify: `src-tauri/src/cli/tui/data.rs`
- Modify: `src-tauri/src/cli/tui/app.rs`
- Modify: `src-tauri/src/cli/tui/mod.rs`
- Modify: `src-tauri/src/cli/i18n.rs`
- Test: `src-tauri/src/cli/tui/ui.rs`
- Test: `src-tauri/src/cli/tui/app.rs`

**Step 1: Write the failing test**

- Add a TUI test that proves the current app still shows a start action when the managed runtime is already running for some other supported app.
- Add a TUI test that proves `OpenCode` shows no proxy action and no misleading “ready/direct” proxy copy.
- Add a TUI/app test that proves `P` toggles the current app only, instead of being blocked by a runtime that belongs to another app.

**Step 2: Run test to verify it fails**

Run: `cargo test home_proxy_dashboard_ -- --nocapture`

Expected: FAIL because the current hero logic still mixes runtime state and current-app state.

**Step 3: Write minimal implementation**

- In `src-tauri/src/cli/tui/data.rs`, keep exposing both:
  - shared runtime facts (`running`, `listen`, `uptime`, `requests`, `active_targets`)
  - current-app facts (`takeover_enabled_for(app)`, `routes_current_app_through_proxy(app)`)
- In `src-tauri/src/cli/tui/app.rs`, change `main_proxy_action()` so supported apps can still emit a start action when the runtime is already up for another app.
- In `src-tauri/src/cli/tui/mod.rs`, keep the worker message shape, but rely on the new service behavior so “enable current app” can reuse the existing managed runtime.
- In `src-tauri/src/cli/i18n.rs`, keep copy app-specific and short:
  - current app on
  - current app off
  - unsupported app

**Step 4: Run test to verify it passes**

Run: `cargo test home_proxy_dashboard_ -- --nocapture`

Expected: PASS.

**Step 5: Run targeted app-action tests**

Run: `cargo test main_proxy_action -- --nocapture`

Expected: PASS.

---

### Task 3: Tighten The Home Card Around Current App Semantics

**Files:**
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Modify: `src-tauri/src/cli/i18n.rs`
- Test: `src-tauri/src/cli/tui/ui.rs`
- Visual check: `ghostty` via `tui-pilot`

**Step 1: Write the failing test**

- Add a rendering test that proves the hero headline and badge reflect the current app’s proxy state, even if another supported app is active in the shared runtime.
- Add a rendering test that proves the lightweight rate animation only pulses when the current app itself is routed through proxy.
- Add a rendering test that proves unsupported apps stay local-only and do not render proxy CTA copy.

**Step 2: Run test to verify it fails**

Run: `cargo test home_proxy_dashboard_ -- --nocapture`

Expected: FAIL because the current hero still partly reflects shared runtime state.

**Step 3: Write minimal implementation**

- In `src-tauri/src/cli/tui/ui.rs`, treat the card as “current app proxy” first and “shared runtime facts” second.
- Keep the current compact layout, but change the decision rules:
  - if current app is routed: show active state and pulse
  - if current app is not routed but the shared runtime is up: show an off/local state for the current app, while still allowing `P` to attach it
  - if unsupported: show local-only with no proxy control hints
- If useful, show a very small hint when another app is active, but keep it to one short line max.

**Step 4: Run test to verify it passes**

Run: `cargo test home_proxy_dashboard_ -- --nocapture`

Expected: PASS.

**Step 5: Run visual verification**

Run the built binary in `ghostty` through `tui-pilot`, capture one supported app with proxy on, one supported app with another app already attached, and `OpenCode`.

Expected: one-card UX stays clear, no misleading start/stop text, no extra control noise.

---

## Final Verification

Run in sequence:

1. `cargo fmt`
2. `cargo test --test proxy_service -- --nocapture`
3. `cargo test --test proxy_takeover -- --nocapture`
4. `cargo test home_proxy_dashboard_ -- --nocapture`
5. `cargo test main_proxy_action -- --nocapture`
6. `cargo build --bin cc-switch`

Expected: PASS.

## Explicitly Deferred

- Separate proxy daemon per app
- Separate listen port per app
- Same-screen three-app dashboard
- `OpenCode` proxy support before upstream has it
- New CLI product surface for per-app proxy management outside TUI
