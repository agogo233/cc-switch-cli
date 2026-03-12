# TUI Mod Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在保持 `tui::run(app_override)` 语义不变的前提下，拆分 `src-tauri/src/cli/tui/mod.rs` 中的事件循环、action 执行和后台 worker 代码。

**Architecture:** 保留 `src-tauri/src/cli/tui/mod.rs` 作为入口壳层，只让它负责模块声明、公共入口和少量粘合逻辑；将 action handler、skills/webdav helper、background system/worker definitions 按职责搬到本地子模块。优先做搬运式重构，不改变消息流、路由、副作用时机和错误处理文案。

**Tech Stack:** Rust, std::sync::mpsc, crossterm, ratatui, crate services layer

---

### Task 1: 抽出 action 执行与辅助 helper

**Files:**
- Modify: `src-tauri/src/cli/tui/mod.rs`
- Create: `src-tauri/src/cli/tui/runtime_actions.rs`
- Create: `src-tauri/src/cli/tui/runtime_skills.rs`

**Step 1: 搬出 action executor**

把 `handle_action` 从 `src-tauri/src/cli/tui/mod.rs` 搬到 `src-tauri/src/cli/tui/runtime_actions.rs`，保留函数签名和调用点语义不变。

**Step 2: 搬出技能导入相关 helper**

把 `scan_unmanaged_skills*`、`open_skills_import_picker*`、`finish_skills_import_with` 等与技能导入流程耦合的 helper 搬到 `src-tauri/src/cli/tui/runtime_skills.rs`。

**Step 3: 编译验证**

Run: `cargo test tui::tests`
Expected: TUI runtime 相关测试继续通过。

### Task 2: 抽出后台 systems 与 worker loops

**Files:**
- Modify: `src-tauri/src/cli/tui/mod.rs`
- Create: `src-tauri/src/cli/tui/runtime_systems.rs`

**Step 1: 搬出 system 类型和 req/msg 定义**

把 `SpeedtestSystem`、`StreamCheckSystem`、`LocalEnvSystem`、`ProxySystem`、`SkillsSystem`、`WebDavSystem`、`UpdateSystem`、`ModelFetchSystem` 以及相关 req/msg enum/struct 挪到 `src-tauri/src/cli/tui/runtime_systems.rs`。

**Step 2: 搬出 start_*_system / *_worker_loop**

把各类 worker 启动函数和线程循环搬到 `src-tauri/src/cli/tui/runtime_systems.rs`，保持 channel 协议与错误处理不变。

**Step 3: 编译验证**

Run: `cargo test tui::tests`
Expected: runtime tests 继续通过。

### Task 3: 收口入口文件并全量验证

**Files:**
- Modify: `src-tauri/src/cli/tui/mod.rs`

**Step 1: 整理入口依赖**

让 `src-tauri/src/cli/tui/mod.rs` 只保留 `run()`、事件循环所需的最少 glue code、模块声明与 re-use。

**Step 2: 格式化**

Run: `cargo fmt`
Expected: 无报错。

**Step 3: TUI 定向测试**

Run: `cargo test tui::`
Expected: 全部 TUI 相关测试通过。

**Step 4: 全量测试**

Run: `cargo test`
Expected: 全量测试通过。
