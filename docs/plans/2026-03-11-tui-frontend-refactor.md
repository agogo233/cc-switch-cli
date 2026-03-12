# TUI Frontend Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在不改变 TUI 语义与交互行为的前提下，拆分超长前端 TUI 模块，让渲染、交互和表单代码按职责收敛到更小的本地模块中。

**Architecture:** 保留 `tui::run`、`ui::render`、`App::on_key`、`FormState` 等现有对外入口不变，只把内部 helper、路由渲染函数、overlay 渲染函数、表单状态/序列化逻辑迁移到 `app/`、`ui/`、`form/` 子模块。优先做“搬运式重构”，避免行为改写和抽象升级。

**Tech Stack:** Rust, ratatui, crossterm, serde_json, toml_edit

---

### Task 1: 拆分 `app.rs` 的类型与按键处理

**Files:**
- Modify: `src-tauri/src/cli/tui/app.rs`
- Create: `src-tauri/src/cli/tui/app/state.rs`
- Create: `src-tauri/src/cli/tui/app/overlay.rs`
- Create: `src-tauri/src/cli/tui/app/editor.rs`
- Create: `src-tauri/src/cli/tui/app/actions.rs`
- Create: `src-tauri/src/cli/tui/app/content_keys.rs`
- Create: `src-tauri/src/cli/tui/app/overlay_keys.rs`
- Create: `src-tauri/src/cli/tui/app/form_keys.rs`
- Create: `src-tauri/src/cli/tui/app/helpers.rs`
- Create: `src-tauri/src/cli/tui/app/tests.rs`

**Step 1: 抽出状态与类型定义**

将 `FilterState`、`Toast`、`Overlay`、`EditorState`、`Action`、`ConfigItem`、`SettingsItem`、`WebDavConfigItem`、`App` 等定义迁移到职责对应的子模块，并在 `app.rs` 中 `pub use` 回原路径。

**Step 2: 抽出路由按键处理**

把 skills/providers/mcp/prompts/config/settings 等内容区按键处理迁移到 `content_keys.rs`，保持 `App::on_content_key` 调用关系不变。

**Step 3: 抽出 overlay / form / editor 按键处理**

把 `on_overlay_key`、`on_form_key`、`on_editor_key` 以及相关 helper 挪到子模块，保留现有 `impl App` API。

**Step 4: 拆出测试模块**

把 `#[cfg(test)]` 测试迁移到 `app/tests.rs`，避免主文件继续膨胀。

**Step 5: 验证编译**

Run: `cargo test app:: --quiet`
Expected: 相关 TUI app 测试通过或至少成功编译到测试阶段。

### Task 2: 拆分 `form.rs` 的 provider / mcp / codex 配置逻辑

**Files:**
- Modify: `src-tauri/src/cli/tui/form.rs`
- Create: `src-tauri/src/cli/tui/form/provider_state.rs`
- Create: `src-tauri/src/cli/tui/form/provider_templates.rs`
- Create: `src-tauri/src/cli/tui/form/provider_json.rs`
- Create: `src-tauri/src/cli/tui/form/mcp.rs`
- Create: `src-tauri/src/cli/tui/form/codex_config.rs`
- Create: `src-tauri/src/cli/tui/form/tests.rs`

**Step 1: 拆分 provider 状态与模板逻辑**

把 `ProviderAddFormState` 的字段访问、模板应用、选择列表等迁移到 `provider_state.rs` / `provider_templates.rs`。

**Step 2: 拆分 provider JSON / TOML 生成逻辑**

把 `to_provider_json_value*`、`apply_provider_json*` 和 Codex TOML 合并/剥离 helper 迁移到 `provider_json.rs` / `codex_config.rs`。

**Step 3: 拆分 MCP 表单逻辑**

把 `McpAddFormState` 迁移到 `mcp.rs`，保留 `FormState::McpAdd` 的原路径。

**Step 4: 拆分测试模块**

把 `#[cfg(test)]` 测试迁移到 `form/tests.rs`。

**Step 5: 验证编译**

Run: `cargo test form:: --quiet`
Expected: 表单相关测试通过或至少成功编译到测试阶段。

### Task 3: 拆分 `ui.rs` 的页面渲染与 overlay 渲染

**Files:**
- Modify: `src-tauri/src/cli/tui/ui.rs`
- Create: `src-tauri/src/cli/tui/ui/shared.rs`
- Create: `src-tauri/src/cli/tui/ui/chrome.rs`
- Create: `src-tauri/src/cli/tui/ui/skills.rs`
- Create: `src-tauri/src/cli/tui/ui/skill_detail.rs`
- Create: `src-tauri/src/cli/tui/ui/editor.rs`
- Create: `src-tauri/src/cli/tui/ui/dashboard.rs`
- Create: `src-tauri/src/cli/tui/ui/management.rs`
- Create: `src-tauri/src/cli/tui/ui/settings.rs`
- Create: `src-tauri/src/cli/tui/ui/status.rs`
- Create: `src-tauri/src/cli/tui/ui/forms/mod.rs`
- Create: `src-tauri/src/cli/tui/ui/forms/shared.rs`
- Create: `src-tauri/src/cli/tui/ui/forms/provider.rs`
- Create: `src-tauri/src/cli/tui/ui/forms/mcp.rs`
- Create: `src-tauri/src/cli/tui/ui/overlay/mod.rs`
- Create: `src-tauri/src/cli/tui/ui/overlay/shared.rs`
- Create: `src-tauri/src/cli/tui/ui/overlay/basic.rs`
- Create: `src-tauri/src/cli/tui/ui/overlay/pickers.rs`
- Create: `src-tauri/src/cli/tui/ui/overlay/status.rs`
- Create: `src-tauri/src/cli/tui/ui/overlay/update.rs`
- Create: `src-tauri/src/cli/tui/ui/tests.rs`

**Step 1: 抽出共享 helper 与页面分组**

把布局、宽度、mask、key bar、header/nav/footer/toast 等共享函数迁移到 `shared.rs` / `chrome.rs` / `status.rs`。

**Step 2: 抽出表单和 editor 渲染**

把 `render_editor`、provider/mcp 表单渲染与其 helper 挪到 `ui/forms/*`。

**Step 3: 抽出 overlay 渲染**

把不同 overlay 变体按 `basic/pickers/status/update` 拆分，保留 `render_overlay` 作为统一入口。

**Step 4: 抽出页面渲染与测试**

把 dashboard / skills / provider&mcp / config&settings 渲染迁移到子模块，并把测试迁移到 `ui/tests.rs`。

**Step 5: 验证编译**

Run: `cargo test ui:: --quiet`
Expected: 渲染相关测试通过或至少成功编译到测试阶段。

### Task 4: 统一格式化与全量验证

**Files:**
- Modify: `src-tauri/src/cli/tui/*.rs`

**Step 1: 运行格式化**

Run: `cargo fmt`
Expected: 无报错。

**Step 2: 运行目标测试**

Run: `cargo test`
Expected: 全量测试通过。

**Step 3: 运行 lint（如时间允许）**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 无 warnings。

### Task 5: 做一次 TUI 可视检查

**Files:**
- Inspect: `src-tauri/src/cli/tui/ui.rs`
- Inspect: `src-tauri/src/cli/tui/app.rs`

**Step 1: 启动 tui-pilot 会话**

Run: 以 `cargo run --bin cc-switch` 启动 TUI。
Expected: 主界面成功渲染。

**Step 2: 采样关键界面**

检查主界面、provider 表单、overlay 其中至少一类界面，确认布局、焦点、边框、提示栏无肉眼回归。

**Step 3: 结束会话并记录结果**

记录是否存在视觉偏差；若有，再做局部修正。
