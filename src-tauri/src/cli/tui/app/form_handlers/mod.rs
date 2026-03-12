use super::*;

mod mcp;
mod provider;
mod tab;

impl App {
    pub(crate) fn on_form_key(&mut self, key: KeyEvent, data: &UiData) -> Action {
        if self.handle_form_tab_key(key) {
            return Action::None;
        }

        if let Some(action) = self.handle_provider_template_key(key, data) {
            return action;
        }

        if let Some(action) = self.handle_mcp_template_key(key) {
            return action;
        }

        if let Some(action) = self.handle_provider_focus_key(key, data) {
            return action;
        }

        if let Some(action) = self.handle_mcp_focus_key(key) {
            return action;
        }

        if is_save_shortcut(key) {
            return self.handle_form_save_shortcut(data);
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.form = None;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_form_save_shortcut(&mut self, data: &UiData) -> Action {
        match self.form.as_ref() {
            Some(FormState::ProviderAdd(_)) => self.build_provider_form_save_action(data),
            Some(FormState::McpAdd(_)) => self.build_mcp_form_save_action(),
            None => Action::None,
        }
    }
}
