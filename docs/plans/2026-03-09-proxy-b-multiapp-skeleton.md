# Proxy B Multi-App Skeleton Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an upstream-aligned multi-app proxy skeleton to the CLI repo, with Claude usable first, Codex/Gemini routes and service structure present, and no live takeover yet.

**Architecture:** Reuse the upstream proxy split as much as possible: `proxy/*` contains server, handlers, forwarder, providers, and streaming conversion; a thin CLI-facing service layer starts and stops the local server and reads existing DB-backed proxy config. Phase B keeps proxy serving and runtime config aligned with upstream, but deliberately leaves takeover, backup/restore, and crash recovery outside the first implementation.

**Tech Stack:** Rust, tokio, axum, tower-http, reqwest, rusqlite, clap, ratatui.

---

### Task 1: Align provider model with upstream `apiFormat`

**Files:**
- Modify: `src-tauri/src/provider.rs`
- Modify: `src-tauri/tests/provider_model_roundtrip.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/provider.rs`

**Step 1: Write the failing test**
- Extend the provider roundtrip test so `ProviderMeta.apiFormat = Some("openai_chat")` survives serialize + deserialize.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test provider_model_roundtrip -- --nocapture`
Expected: FAIL because `api_format` field does not exist in `ProviderMeta`.

**Step 3: Write minimal implementation**
- Add `api_format: Option<String>` to `ProviderMeta` with `#[serde(rename = "apiFormat")]`.
- Keep field order and naming close to upstream.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test provider_model_roundtrip -- --nocapture`
Expected: PASS.

**Step 5: Commit**
```bash
git add src-tauri/src/provider.rs src-tauri/tests/provider_model_roundtrip.rs
git commit -m "feat(proxy): align provider apiFormat metadata"
```

### Task 2: Add proxy runtime dependencies and module skeleton

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/proxy/mod.rs`
- Create/Modify: `src-tauri/src/proxy/*.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/proxy/`

**Step 1: Write the failing test**
- Add a focused smoke test that tries to construct the proxy server/service and asserts the server is initially stopped.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test proxy_server -- --nocapture`
Expected: FAIL because server/service types are missing.

**Step 3: Write minimal implementation**
- Add `axum`, `tower-http`, `async-stream`, `bytes`, and any missing upstream-compatible dependencies.
- Expand `proxy/mod.rs` toward the upstream module structure.
- Bring over the smallest set of upstream modules needed to compile a server skeleton and route table.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test proxy_server -- --nocapture`
Expected: PASS.

**Step 5: Commit**
```bash
git add src-tauri/Cargo.toml src-tauri/src/proxy
git commit -m "feat(proxy): scaffold upstream-aligned runtime core"
```

### Task 3: Add CLI service wrapper without takeover

**Files:**
- Create: `src-tauri/src/services/proxy.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/main.rs`
- Check: `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/services/proxy.rs`

**Step 1: Write the failing test**
- Add a service-level test that starts the proxy service, reads status, then stops it.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test proxy_service -- --nocapture`
Expected: FAIL because `ProxyService` is missing.

**Step 3: Write minimal implementation**
- Introduce a CLI-safe `ProxyService` that wraps `ProxyServer`.
- Implement `start`, `stop`, `get_status`, `is_running`, and config reads/updates.
- Explicitly defer `start_with_takeover`, restore, and per-app takeover toggles.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test proxy_service -- --nocapture`
Expected: PASS.

**Step 5: Commit**
```bash
git add src-tauri/src/services/proxy.rs src-tauri/src/services/mod.rs src-tauri/src/lib.rs src-tauri/src/main.rs
git commit -m "feat(proxy): add cli proxy service without takeover"
```

### Task 4: Add multi-app route skeleton and Claude-first forwarding

**Files:**
- Modify/Create: `src-tauri/src/proxy/server.rs`
- Modify/Create: `src-tauri/src/proxy/handlers.rs`
- Modify/Create: `src-tauri/src/proxy/forwarder.rs`
- Modify/Create: `src-tauri/src/proxy/providers/*.rs`
- Modify/Create: `src-tauri/src/proxy/provider_router.rs`
- Check: upstream proxy files under `/Users/saladday/dev/cc-switch-cli/.upstream/cc-switch/src-tauri/src/proxy/`

**Step 1: Write the failing test**
- Add a focused integration test for Claude `apiFormat=openai_chat`: incoming `/v1/messages` should be routed through the Claude adapter, rewritten to `/v1/chat/completions`, and converted back to Anthropic format.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test proxy_claude_openai_chat -- --nocapture`
Expected: FAIL because route/adapter/transform chain is incomplete.

**Step 3: Write minimal implementation**
- Bring in upstream route layout for Claude, Codex, and Gemini.
- Make Claude fully functional first, including request/response transform and streaming conversion.
- Keep Codex/Gemini handlers wired and compile-ready even if first pass only supports passthrough/basic routing.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test proxy_claude_openai_chat -- --nocapture`
Expected: PASS.

**Step 5: Commit**
```bash
git add src-tauri/src/proxy src-tauri/tests
git commit -m "feat(proxy): add multi-app routing with claude-first transform"
```

### Task 5: Add `proxy serve` debug entry and TUI-facing discovery hooks

**Files:**
- Modify: `src-tauri/src/cli/mod.rs`
- Create: `src-tauri/src/cli/commands/proxy.rs`
- Modify: `src-tauri/src/cli/commands/mod.rs`
- Modify: `src-tauri/src/cli/i18n.rs`
- Modify: `src-tauri/src/cli/interactive/`
- Modify: `src-tauri/src/cli/tui/`

**Step 1: Write the failing test**
- Add a CLI command parsing test for `cc-switch proxy serve`.
- Add a small TUI data/action test proving proxy settings can be surfaced.

**Step 2: Run test to verify it fails**
Run: `cd src-tauri && cargo test proxy_command -- --nocapture`
Expected: FAIL because command and TUI entry points are missing.

**Step 3: Write minimal implementation**
- Add `proxy serve`, `proxy status`, and `proxy stop` command entry points for local debugging.
- Add TUI navigation and settings entry points, but keep UX thin in the first pass.
- Keep user-facing strings in `i18n.rs`.

**Step 4: Run test to verify it passes**
Run: `cd src-tauri && cargo test proxy_command -- --nocapture`
Expected: PASS.

**Step 5: Commit**
```bash
git add src-tauri/src/cli src-tauri/tests
git commit -m "feat(proxy): expose proxy controls in cli and tui"
```

### Task 6: Verify Phase B and document the follow-up to Phase C

**Files:**
- Modify: `README.md` (if needed)
- Modify: `README_ZH.md` (if needed)
- Modify: `docs/plans/2026-03-09-proxy-b-multiapp-skeleton.md`

**Step 1: Run focused tests**
Run: `cd src-tauri && cargo test provider_model_roundtrip proxy_ proxy_service proxy_command -- --nocapture`
Expected: PASS.

**Step 2: Run full suite**
Run: `cd src-tauri && cargo test`
Expected: PASS.

**Step 3: Record Phase C boundary**
- Confirm that takeover remains intentionally out of scope.
- Note that `C` will add live backup/restore, takeover flags, placeholder token writes, and recovery using the same proxy core.

**Step 4: Commit**
```bash
git add docs/plans/2026-03-09-proxy-b-multiapp-skeleton.md README.md README_ZH.md
git commit -m "docs: capture proxy phase b and c boundary"
```


## Status Snapshot (2026-03-09)

### Implemented in Solution B

- Provider metadata now aligns with upstream `apiFormat`, including `openai_chat` roundtrip support.
- Added a thin CLI-side `ProxyService` that starts/stops the local server and reads DB-backed proxy config without introducing takeover behavior.
- Added upstream-shaped multi-app routes for Claude, Codex, and Gemini.
- Claude supports `apiFormat=openai_chat` request/response conversion and streaming SSE conversion back to Anthropic format.
- Codex supports passthrough for `/v1/chat/completions` and `/v1/responses`.
- Gemini supports passthrough for `/v1beta/*` routes.
- Added `cc-switch proxy show` and foreground `cc-switch proxy serve` plus a TUI `Config -> Local Proxy` discovery entry. Both now explain the manual Claude hookup path for issue #49.

### Intentional Phase B Boundary

- No live takeover, backup/restore, placeholder token writes into Claude config files, or crash-recovery flow yet. Those remain Phase C work.
- No daemonized cross-process `proxy status` / `proxy stop` command yet. Phase B keeps the proxy as a foreground debug process so behavior stays explicit.
- Provider resolution still uses the current provider stored in the DB for each app. We intentionally did not bring in upstream `handler_context` / `provider_router` / failover lifecycle yet, because this phase is meant to keep issue #49 usable without adding error switching.

### Verification Notes

- Focused coverage now includes Claude non-stream transform, Claude streaming transform, Codex responses passthrough, Gemini passthrough, provider metadata roundtrip, proxy service lifecycle, CLI parsing, and TUI proxy overlay discovery.
- Keep file sizes intentionally small: the new/edited proxy files remain below the repository's ~500-line soft limit.
