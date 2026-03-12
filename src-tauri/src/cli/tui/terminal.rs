use std::io::{self, Stdout};
use std::sync::Arc;

use crossterm::{
    cursor,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::Size;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::error::AppError;

pub struct TuiTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}

pub struct PanicRestoreHookGuard {
    previous: Option<Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>>,
}

impl PanicRestoreHookGuard {
    pub fn install() -> Self {
        let previous = std::panic::take_hook();
        let previous: Arc<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static> =
            previous.into();
        let previous_for_hook = previous.clone();

        std::panic::set_hook(Box::new(move |info| {
            let mut stdout = io::stdout();
            let _ = restore_stdout_best_effort(&mut stdout);
            previous_for_hook(info);
        }));

        Self {
            previous: Some(previous),
        }
    }
}

impl Drop for PanicRestoreHookGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::panic::set_hook(Box::new(move |info| previous(info)));
        }
    }
}

fn record_err(first_err: &mut Option<AppError>, e: impl ToString) {
    if first_err.is_none() {
        *first_err = Some(AppError::localized(
            "tui_terminal_error",
            format!("终端错误: {}", e.to_string()),
            format!("Terminal error: {}", e.to_string()),
        ));
    }
}

fn restore_stdout_best_effort(stdout: &mut Stdout) -> Result<(), AppError> {
    let mut first_err: Option<AppError> = None;

    if let Err(e) = disable_raw_mode() {
        record_err(&mut first_err, e);
    }

    if let Err(e) = execute!(
        stdout,
        cursor::Show,
        LeaveAlternateScreen,
        DisableMouseCapture
    ) {
        record_err(&mut first_err, e);
    }

    if let Some(err) = first_err {
        Err(err)
    } else {
        Ok(())
    }
}

impl TuiTerminal {
    pub fn new() -> Result<Self, AppError> {
        let mut stdout = io::stdout();
        enable_raw_mode().map_err(|e| {
            AppError::localized(
                "tui_terminal_error",
                format!("终端错误: {}", e.to_string()),
                format!("Terminal error: {}", e.to_string()),
            )
        })?;
        if let Err(e) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        ) {
            let _ = restore_stdout_best_effort(&mut stdout);
            return Err(AppError::localized(
                "tui_terminal_error",
                format!("终端错误: {}", e.to_string()),
                format!("Terminal error: {}", e.to_string()),
            ));
        }

        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(e) => {
                let mut stdout = io::stdout();
                let _ = restore_stdout_best_effort(&mut stdout);
                return Err(AppError::localized(
                    "tui_terminal_error",
                    format!("终端错误: {}", e.to_string()),
                    format!("Terminal error: {}", e.to_string()),
                ));
            }
        };

        Ok(Self {
            terminal,
            active: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Result<Self, AppError> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend).map_err(|e| {
            AppError::localized(
                "tui_terminal_error",
                format!("终端错误: {}", e),
                format!("Terminal error: {}", e),
            )
        })?;

        Ok(Self {
            terminal,
            active: false,
        })
    }

    pub fn draw<F>(&mut self, f: F) -> Result<(), AppError>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(f).map(|_| ()).map_err(|e| {
            AppError::localized(
                "tui_terminal_error",
                format!("终端错误: {}", e.to_string()),
                format!("Terminal error: {}", e.to_string()),
            )
        })
    }

    pub fn size(&self) -> Result<Size, AppError> {
        self.terminal.size().map_err(|e| {
            AppError::localized(
                "tui_terminal_error",
                format!("终端错误: {}", e.to_string()),
                format!("Terminal error: {}", e.to_string()),
            )
        })
    }

    pub fn with_terminal_restored<T>(
        &mut self,
        f: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.restore_best_effort()?;

        struct ReactivateOnDrop<'a> {
            terminal: &'a mut TuiTerminal,
            reactivated: bool,
        }

        impl Drop for ReactivateOnDrop<'_> {
            fn drop(&mut self) {
                if self.reactivated {
                    return;
                }
                let _ = self.terminal.activate_best_effort();
            }
        }

        let mut guard = ReactivateOnDrop {
            terminal: self,
            reactivated: false,
        };

        let result = f();
        let activate_result = guard.terminal.activate_best_effort();
        if activate_result.is_ok() {
            guard.reactivated = true;
        }
        activate_result?;

        result
    }

    pub fn restore_best_effort(&mut self) -> Result<(), AppError> {
        if !self.active {
            return Ok(());
        }

        let mut first_err: Option<AppError> = None;

        if let Err(e) = disable_raw_mode() {
            record_err(&mut first_err, e);
        }

        if let Err(e) = execute!(
            self.terminal.backend_mut(),
            cursor::Show,
            LeaveAlternateScreen,
            DisableMouseCapture
        ) {
            record_err(&mut first_err, e);
        }
        let _ = self.terminal.show_cursor();

        if let Some(err) = first_err {
            Err(err)
        } else {
            self.active = false;
            Ok(())
        }
    }

    pub fn activate_best_effort(&mut self) -> Result<(), AppError> {
        if self.active {
            return Ok(());
        }

        let mut first_err: Option<AppError> = None;

        if let Err(e) = enable_raw_mode() {
            record_err(&mut first_err, e);
        }

        if let Err(e) = execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        ) {
            record_err(&mut first_err, e);
        }

        if let Err(e) = self.terminal.clear() {
            record_err(&mut first_err, e);
        }

        if let Some(err) = first_err {
            Err(err)
        } else {
            self.active = true;
            Ok(())
        }
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = self.restore_best_effort();
    }
}
