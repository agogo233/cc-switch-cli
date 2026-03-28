use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::i18n::texts;
use crate::config::write_json_file;
use crate::error::AppError;
use crate::provider::Provider;
use serde_json::json;

use super::super::app::ToastKind;
use super::super::terminal::TuiTerminal;
use super::RuntimeActionContext;

#[derive(Debug)]
struct PreparedClaudeLaunch {
    executable: PathBuf,
    settings_path: PathBuf,
}

pub(super) fn launch(ctx: &mut RuntimeActionContext<'_>, id: String) -> Result<(), AppError> {
    launch_with(
        ctx,
        id,
        &std::env::temp_dir(),
        resolve_claude_binary,
        handoff_to_claude,
    )
}

fn launch_with<Resolve, Handoff>(
    ctx: &mut RuntimeActionContext<'_>,
    id: String,
    temp_dir: &Path,
    resolve_claude_binary: Resolve,
    handoff: Handoff,
) -> Result<(), AppError>
where
    Resolve: FnOnce() -> Result<PathBuf, AppError>,
    Handoff: FnOnce(&mut TuiTerminal, &PreparedClaudeLaunch) -> Result<(), AppError>,
{
    if let Err(err) = try_launch_with(ctx, &id, temp_dir, resolve_claude_binary, handoff) {
        ctx.app.push_toast(
            texts::tui_temp_launch_failed(&err.to_string()),
            ToastKind::Error,
        );
    }
    Ok(())
}

fn prepare_launch_with<Resolve>(
    provider: &Provider,
    temp_dir: &Path,
    resolve_claude_binary: Resolve,
) -> Result<PreparedClaudeLaunch, AppError>
where
    Resolve: FnOnce() -> Result<PathBuf, AppError>,
{
    let executable = resolve_claude_binary()?;
    let env = provider
        .settings_config
        .get("env")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            AppError::localized(
                "claude.temp_launch_missing_env",
                format!("供应商 {} 缺少有效的 env 配置。", provider.id),
                format!("Provider {} is missing a valid env object.", provider.id),
            )
        })?;
    let settings_path = write_temp_settings_file(temp_dir, provider, &json!({ "env": env }))?;

    Ok(PreparedClaudeLaunch {
        executable,
        settings_path,
    })
}

fn resolve_claude_binary() -> Result<PathBuf, AppError> {
    which::which("claude").map_err(|_| {
        AppError::localized(
            "claude.temp_launch_missing_binary",
            "未找到 claude 命令，请先安装 Claude CLI。".to_string(),
            "Could not find `claude` in PATH. Install Claude CLI first.".to_string(),
        )
    })
}

#[cfg(unix)]
fn handoff_to_claude(
    terminal: &mut TuiTerminal,
    prepared: &PreparedClaudeLaunch,
) -> Result<(), AppError> {
    use std::os::unix::process::CommandExt;

    terminal.restore_best_effort()?;

    let exec_err = std::process::Command::new(&prepared.executable)
        .arg("--settings")
        .arg(&prepared.settings_path)
        .exec();

    match terminal.activate_best_effort() {
        Ok(()) => Err(AppError::localized(
            "claude.temp_launch_exec_failed",
            format!("启动 Claude 失败: {exec_err}"),
            format!("Failed to launch Claude: {exec_err}"),
        )),
        Err(reactivate_err) => Err(AppError::localized(
            "claude.temp_launch_exec_failed",
            format!("启动 Claude 失败: {exec_err}；重新激活 TUI 失败: {reactivate_err}"),
            format!("Failed to launch Claude: {exec_err}; also failed to reactivate the TUI: {reactivate_err}"),
        )),
    }
}

#[cfg(not(unix))]
fn handoff_to_claude(
    _terminal: &mut TuiTerminal,
    _prepared: &PreparedClaudeLaunch,
) -> Result<(), AppError> {
    Err(AppError::localized(
        "claude.temp_launch_unsupported_platform",
        "当前平台暂不支持在当前终端临时启动 Claude。".to_string(),
        "Temporary Claude launch in the current terminal is not supported on this platform."
            .to_string(),
    ))
}

fn try_launch_with<Resolve, Handoff>(
    ctx: &mut RuntimeActionContext<'_>,
    id: &str,
    temp_dir: &Path,
    resolve_claude_binary: Resolve,
    handoff: Handoff,
) -> Result<(), AppError>
where
    Resolve: FnOnce() -> Result<PathBuf, AppError>,
    Handoff: FnOnce(&mut TuiTerminal, &PreparedClaudeLaunch) -> Result<(), AppError>,
{
    let provider = ctx
        .data
        .providers
        .rows
        .iter()
        .find(|row| row.id == id)
        .map(|row| row.provider.clone())
        .ok_or_else(|| {
            AppError::localized(
                "claude.temp_launch_provider_not_found",
                format!("未找到选中的供应商: {id}"),
                format!("Selected provider not found: {id}"),
            )
        })?;
    let prepared = prepare_launch_with(&provider, temp_dir, resolve_claude_binary)?;

    handoff(ctx.terminal, &prepared)
}

fn write_temp_settings_file(
    temp_dir: &Path,
    provider: &Provider,
    settings: &serde_json::Value,
) -> Result<PathBuf, AppError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "cc-switch-claude-{}-{}-{timestamp}.json",
        sanitize_filename_fragment(&provider.id),
        std::process::id()
    );
    let path = temp_dir.join(filename);

    write_json_file(&path, settings)?;
    Ok(path)
}

fn sanitize_filename_fragment(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect();
    if sanitized.is_empty() {
        "provider".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppType;
    use crate::cli::tui::app::{App, ToastKind};
    use crate::cli::tui::data::{ProviderRow, ProvidersSnapshot, UiData};
    use crate::cli::tui::runtime_systems::RequestTracker;
    use crate::provider::Provider;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    struct LaunchFixture {
        terminal: TuiTerminal,
        app: App,
        data: UiData,
        proxy_loading: RequestTracker,
        webdav_loading: RequestTracker,
        update_check: RequestTracker,
    }

    impl LaunchFixture {
        fn new(current_id: &str, rows: Vec<ProviderRow>) -> Self {
            Self {
                terminal: TuiTerminal::new_for_test().expect("create test terminal"),
                app: App::new(Some(AppType::Claude)),
                data: UiData {
                    providers: ProvidersSnapshot {
                        current_id: current_id.to_string(),
                        rows,
                    },
                    ..UiData::default()
                },
                proxy_loading: RequestTracker::default(),
                webdav_loading: RequestTracker::default(),
                update_check: RequestTracker::default(),
            }
        }

        fn ctx(&mut self) -> RuntimeActionContext<'_> {
            RuntimeActionContext {
                terminal: &mut self.terminal,
                app: &mut self.app,
                data: &mut self.data,
                speedtest_req_tx: None,
                stream_check_req_tx: None,
                skills_req_tx: None,
                proxy_req_tx: None,
                proxy_loading: &mut self.proxy_loading,
                local_env_req_tx: None,
                webdav_req_tx: None,
                webdav_loading: &mut self.webdav_loading,
                update_req_tx: None,
                update_check: &mut self.update_check,
                model_fetch_req_tx: None,
            }
        }
    }

    fn provider_row(id: &str, env: Value) -> ProviderRow {
        ProviderRow {
            id: id.to_string(),
            provider: Provider::with_id(
                id.to_string(),
                format!("Provider {id}"),
                json!({ "env": env }),
                None,
            ),
            api_url: Some("https://api.example.com".to_string()),
            is_current: false,
            is_in_config: true,
            is_saved: true,
            is_default_model: false,
            primary_model_id: None,
            default_model_id: None,
        }
    }

    #[test]
    fn prepare_launch_writes_claude_env_settings_file() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let provider = Provider::with_id(
            "demo".to_string(),
            "Demo".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "sk-demo",
                    "ANTHROPIC_BASE_URL": "https://api.example.com"
                }
            }),
            None,
        );

        let prepared = prepare_launch_with(&provider, temp_dir.path(), || {
            Ok(PathBuf::from("/usr/bin/claude"))
        })
        .expect("prepare launch");

        assert_eq!(prepared.executable, PathBuf::from("/usr/bin/claude"));
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(&prepared.settings_path).expect("read temp settings"),
        )
        .expect("parse temp settings");
        assert_eq!(
            written,
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "sk-demo",
                    "ANTHROPIC_BASE_URL": "https://api.example.com"
                }
            })
        );
    }

    #[test]
    fn missing_claude_binary_reports_an_error() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let provider = Provider::with_id(
            "demo".to_string(),
            "Demo".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "sk-demo"
                }
            }),
            None,
        );

        let err = prepare_launch_with(&provider, temp_dir.path(), || {
            Err(AppError::Message("claude binary is missing".to_string()))
        })
        .expect_err("missing binary should fail");

        assert!(
            err.to_string().contains("claude"),
            "error should mention claude: {err}"
        );
    }

    #[test]
    fn launch_failure_does_not_switch_current_provider_and_surfaces_a_toast() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let mut fixture = LaunchFixture::new(
            "current",
            vec![
                provider_row(
                    "current",
                    json!({
                        "ANTHROPIC_AUTH_TOKEN": "sk-current"
                    }),
                ),
                provider_row(
                    "candidate",
                    json!({
                        "ANTHROPIC_AUTH_TOKEN": "sk-candidate"
                    }),
                ),
            ],
        );

        launch_with(
            &mut fixture.ctx(),
            "candidate".to_string(),
            temp_dir.path(),
            || Ok(PathBuf::from("/usr/bin/claude")),
            |_, _| Err(AppError::Message("launch exploded".to_string())),
        )
        .expect("launch failure should stay in the TUI");

        assert_eq!(fixture.data.providers.current_id, "current");
        assert!(
            matches!(
                fixture.app.toast.as_ref(),
                Some(toast)
                    if toast.kind == ToastKind::Error
                        && toast.message == texts::tui_temp_launch_failed("launch exploded")
            ),
            "expected temp launch failure toast, got {:?}",
            fixture.app.toast
        );
    }
}
