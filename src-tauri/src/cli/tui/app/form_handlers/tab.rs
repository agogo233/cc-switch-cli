use super::*;

impl App {
    pub(super) fn handle_form_tab_key(&mut self, key: KeyEvent) -> bool {
        if !matches!(key.code, KeyCode::Tab) {
            return false;
        }

        let Some(form) = self.form.as_mut() else {
            return false;
        };

        match form {
            FormState::ProviderAdd(provider) => {
                if matches!(provider.app_type, AppType::Codex) {
                    match (
                        &provider.mode,
                        provider.focus,
                        provider.codex_preview_section,
                    ) {
                        (FormMode::Add, FormFocus::Templates, _) => {
                            provider.focus = FormFocus::Fields;
                        }
                        (FormMode::Add, FormFocus::Fields, _) => {
                            provider.focus = FormFocus::JsonPreview;
                            provider.codex_preview_section = form::CodexPreviewSection::Auth;
                        }
                        (
                            FormMode::Add,
                            FormFocus::JsonPreview,
                            form::CodexPreviewSection::Auth,
                        ) => {
                            provider.focus = FormFocus::JsonPreview;
                            provider.codex_preview_section = form::CodexPreviewSection::Config;
                        }
                        (
                            FormMode::Add,
                            FormFocus::JsonPreview,
                            form::CodexPreviewSection::Config,
                        ) => {
                            provider.focus = FormFocus::Templates;
                        }
                        (FormMode::Edit { .. }, FormFocus::Fields, _) => {
                            provider.focus = FormFocus::JsonPreview;
                            provider.codex_preview_section = form::CodexPreviewSection::Auth;
                        }
                        (
                            FormMode::Edit { .. },
                            FormFocus::JsonPreview,
                            form::CodexPreviewSection::Auth,
                        ) => {
                            provider.focus = FormFocus::JsonPreview;
                            provider.codex_preview_section = form::CodexPreviewSection::Config;
                        }
                        (
                            FormMode::Edit { .. },
                            FormFocus::JsonPreview,
                            form::CodexPreviewSection::Config,
                        ) => {
                            provider.focus = FormFocus::Fields;
                        }
                        (FormMode::Edit { .. }, FormFocus::Templates, _) => {
                            provider.focus = FormFocus::Fields;
                        }
                    }
                } else {
                    provider.focus = match (&provider.mode, provider.focus) {
                        (FormMode::Add, FormFocus::Templates) => FormFocus::Fields,
                        (FormMode::Add, FormFocus::Fields) => FormFocus::JsonPreview,
                        (FormMode::Add, FormFocus::JsonPreview) => FormFocus::Templates,
                        (FormMode::Edit { .. }, FormFocus::Fields) => FormFocus::JsonPreview,
                        (FormMode::Edit { .. }, FormFocus::JsonPreview) => FormFocus::Fields,
                        (FormMode::Edit { .. }, FormFocus::Templates) => FormFocus::Fields,
                    };
                }
            }
            FormState::McpAdd(mcp) => {
                mcp.focus = match (&mcp.mode, mcp.focus) {
                    (FormMode::Add, FormFocus::Templates) => FormFocus::Fields,
                    (FormMode::Add, FormFocus::Fields) => FormFocus::JsonPreview,
                    (FormMode::Add, FormFocus::JsonPreview) => FormFocus::Templates,
                    (FormMode::Edit { .. }, FormFocus::Fields) => FormFocus::JsonPreview,
                    (FormMode::Edit { .. }, FormFocus::JsonPreview) => FormFocus::Fields,
                    (FormMode::Edit { .. }, FormFocus::Templates) => FormFocus::Fields,
                };
            }
        }

        true
    }
}
