use super::*;

impl App {
    pub(crate) fn on_editor_key(&mut self, key: KeyEvent) -> Action {
        let viewport = self.editor_viewport_size();
        let jump_rows = viewport.height as usize;

        let Some(editor) = &mut self.editor else {
            return Action::None;
        };

        if is_save_shortcut(key) {
            return Action::EditorSubmit {
                submit: editor.submit.clone(),
                content: editor.text(),
            };
        }

        if is_open_external_editor_shortcut(key) {
            return Action::EditorOpenExternal;
        }

        match key.code {
            KeyCode::Esc => {
                if editor.is_dirty() {
                    self.overlay = Overlay::Confirm(ConfirmOverlay {
                        title: texts::tui_editor_save_before_close_title().to_string(),
                        message: texts::tui_editor_save_before_close_message().to_string(),
                        action: ConfirmAction::EditorSaveBeforeClose,
                    });
                    Action::None
                } else {
                    self.editor = None;
                    Action::None
                }
            }
            KeyCode::Up => {
                editor.cursor_row = editor.cursor_row.saturating_sub(1);
                editor.cursor_col = editor
                    .cursor_col
                    .min(editor.line_len_chars(editor.cursor_row));
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Down => {
                if !editor.lines.is_empty() {
                    editor.cursor_row = (editor.cursor_row + 1).min(editor.lines.len() - 1);
                }
                editor.cursor_col = editor
                    .cursor_col
                    .min(editor.line_len_chars(editor.cursor_row));
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Left => {
                if editor.cursor_col > 0 {
                    editor.cursor_col -= 1;
                } else if editor.cursor_row > 0 {
                    editor.cursor_row -= 1;
                    editor.cursor_col = editor.line_len_chars(editor.cursor_row);
                }
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Right => {
                let line_len = editor.line_len_chars(editor.cursor_row);
                if editor.cursor_col < line_len {
                    editor.cursor_col += 1;
                } else if editor.cursor_row + 1 < editor.lines.len() {
                    editor.cursor_row += 1;
                    editor.cursor_col = 0;
                }
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Home => {
                editor.cursor_col = 0;
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::End => {
                editor.cursor_col = editor.line_len_chars(editor.cursor_row);
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::PageUp => {
                editor.scroll = editor.scroll.saturating_sub(jump_rows);
                editor.cursor_row = editor.cursor_row.saturating_sub(jump_rows);
                editor.cursor_col = editor
                    .cursor_col
                    .min(editor.line_len_chars(editor.cursor_row));
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::PageDown => {
                if !editor.lines.is_empty() {
                    editor.scroll = (editor.scroll + jump_rows).min(editor.lines.len() - 1);
                    editor.cursor_row = (editor.cursor_row + jump_rows).min(editor.lines.len() - 1);
                    editor.cursor_col = editor
                        .cursor_col
                        .min(editor.line_len_chars(editor.cursor_row));
                }
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Backspace => {
                editor.backspace();
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Delete => {
                editor.delete();
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Enter => {
                editor.newline();
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Tab => {
                editor.insert_str("  ");
                editor.ensure_cursor_visible(viewport);
                Action::None
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    editor.insert_char(c);
                    editor.ensure_cursor_visible(viewport);
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    pub(crate) fn editor_viewport_size(&self) -> Size {
        // Matches `render()` + `render_content()` + `render_editor()` layout math in `ui.rs`.
        let mut width = self.last_size.width.saturating_sub(30);
        let mut height = self.last_size.height.saturating_sub(3).saturating_sub(1);

        if self.filter.active || !self.filter.buffer.trim().is_empty() {
            height = height.saturating_sub(5);
        }

        // render_editor:
        // - outer borders (2)
        // - key bar row (1)
        // - field borders (2)
        width = width.saturating_sub(2).saturating_sub(2);
        height = height.saturating_sub(2).saturating_sub(1).saturating_sub(2);

        Size {
            width: width.max(1),
            height: height.max(1),
        }
    }
}
