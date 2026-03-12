# Proxy B Single-Provider UX Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep the local proxy and issue #49 transform path, but make it easy to configure from the maintained TUI without adding failover.

**Architecture:** Reuse the current Phase B proxy core and add only the smallest user-facing pieces: a Claude `apiFormat` toggle in the TUI provider form, clearer proxy setup guidance, and more obvious issue #49 setup hints. Keep runtime behavior single-provider and DB-driven, with no provider router or automatic failover lifecycle.

**Tech Stack:** Rust, tokio, ratatui, serde_json, existing TUI form/app layers, proxy core modules.

---

### Task 1: Add failing Claude apiFormat form tests

**Files:**
- Modify: `src-tauri/src/cli/tui/form.rs`

**Step 1: Write the failing test**
- Add a TUI form test that sets Claude API format to `openai_chat` and expects `meta.apiFormat = "openai_chat"` in the serialized provider JSON.
- Add a second test that loads a provider with `meta.apiFormat = "openai_chat"` and expects the form to restore that value.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test provider_add_form_claude_api_format -- --nocapture`
Expected: FAIL because Claude form state has no `apiFormat` field yet.

**Step 3: Write minimal implementation**
- Add a Claude-only API format field to the provider form state.
- Serialize it through `meta.apiFormat`.
- Restore it from existing providers.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test provider_add_form_claude_api_format -- --nocapture`
Expected: PASS.

### Task 2: Add failing proxy help overlay test

**Files:**
- Modify: `src-tauri/src/cli/tui/app.rs`

**Step 1: Write the failing test**
- Extend the proxy help overlay test to assert it includes a manual issue #49 setup hint, including the local proxy command and Claude env wiring.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test config_proxy_item_opens_proxy_help_overlay -- --nocapture`
Expected: FAIL because the overlay does not yet include the manual env guidance.

**Step 3: Write minimal implementation**
- Add short TUI help lines that explain how to point Claude Code to the local proxy without takeover.
- Keep the wording explicit that this is a foreground/manual setup.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test config_proxy_item_opens_proxy_help_overlay -- --nocapture`
Expected: PASS.

### Task 3: Surface the setting in provider detail and proxy CLI text

**Files:**
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Modify: `src-tauri/src/cli/commands/proxy.rs`
- Modify: `src-tauri/src/cli/i18n.rs`

**Step 1: Write the failing test**
- Add focused tests around the field/value renderer or provider detail output to prove Claude `openai_chat` is visible.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test provider_field_label_and_value -- --nocapture`
Expected: FAIL because no Claude API format row exists.

**Step 3: Write minimal implementation**
- Add the field label/value rendering.
- Show `apiFormat` in Claude provider detail.
- Keep `proxy show` and proxy help wording aligned with the manual setup path.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test provider_field_label_and_value -- --nocapture`
Expected: PASS.

### Task 4: Verify and document the single-provider boundary

**Files:**
- Modify: `docs/plans/2026-03-09-proxy-b-multiapp-skeleton.md`
- Modify: `docs/plans/2026-03-09-proxy-b-single-provider-ux.md`

**Step 1: Run focused tests**
Run: `cd src-tauri && cargo test provider_add_form_claude_api_format config_proxy_item_opens_proxy_help_overlay proxy_ -- --nocapture`
Expected: PASS.

**Step 2: Run full suite**
Run: `cd src-tauri && cargo test`
Expected: PASS.

**Step 3: Record the scope**
- Note that proxy stays.
- Note that failover remains intentionally out of scope.
- Note that this path is designed to make issue #49 easy to configure rather than to reproduce upstream's entire lifecycle stack.

## Status Snapshot (2026-03-09)

### Implemented

- Claude provider form now exposes `apiFormat`, with `anthropic` and `openai_chat` options, and persists the canonical upstream-style `meta.apiFormat` value.
- Claude provider edit/detail views now show the chosen API format, so the issue #49 path is visible from the maintained TUI.
- `Config -> Local Proxy` now includes a manual Claude hookup guide: run `cc-switch proxy serve`, point `ANTHROPIC_BASE_URL` at the local proxy, and use a placeholder auth token.
- `cc-switch proxy show` now mirrors that same manual setup path, so the foreground CLI and TUI tell the same story.

### Boundary

- This pass keeps the proxy and the Claude OpenAI Chat -> Anthropic conversion path.
- This pass does not add failover, automatic error switching, takeover, backup/restore, or crash recovery.
- Provider selection still comes from the current provider stored in the database for each app. That stays simple on purpose so the backend can remain close to the current upstream proxy core without pulling in the whole lifecycle stack.

### Verification

- Focused tests already cover Claude `apiFormat` form roundtrip, proxy help overlay guidance, and provider detail rendering.
- Full formatting and test verification should be rerun after the final doc/CLI wording changes in this worktree.
