use super::*;

const PROXY_ACTIVITY_WINDOW: usize = 48;
const PROXY_ACTIVITY_POLL_INTERVAL_TICKS: u64 = 5;

impl App {
    pub fn new(app_override: Option<AppType>) -> Self {
        let app_type = app_override.unwrap_or(AppType::Claude);
        Self {
            app_type,
            route: Route::Main,
            route_stack: Vec::new(),
            focus: Focus::Nav,
            nav_idx: 0,
            filter: FilterState::new(),
            editor: None,
            form: None,
            overlay: Overlay::None,
            toast: None,
            should_quit: false,
            last_size: Size::new(0, 0),
            tick: 0,
            proxy_input_activity_samples: Vec::new(),
            proxy_output_activity_samples: Vec::new(),
            proxy_activity_last_input_tokens: None,
            proxy_activity_last_output_tokens: None,
            proxy_visual_state: None,
            proxy_visual_transition: None,
            local_env_results: Vec::new(),
            local_env_loading: true,
            provider_idx: 0,
            mcp_idx: 0,
            prompt_idx: 0,
            skills_idx: 0,
            skills_discover_idx: 0,
            skills_repo_idx: 0,
            skills_unmanaged_idx: 0,
            skills_discover_results: Vec::new(),
            skills_discover_query: String::new(),
            skills_unmanaged_results: Vec::new(),
            skills_unmanaged_selected: HashSet::new(),
            config_idx: 0,
            config_webdav_idx: 0,
            webdav_quick_setup_username: None,
            language_idx: 0,
            settings_idx: 0,
            settings_proxy_idx: 0,
        }
    }

    pub fn nav_item(&self) -> NavItem {
        NavItem::ALL
            .get(self.nav_idx)
            .copied()
            .unwrap_or(NavItem::Main)
    }

    pub(crate) fn nav_item_for_route(route: &Route) -> NavItem {
        match route {
            Route::Main => NavItem::Main,
            Route::Providers | Route::ProviderDetail { .. } => NavItem::Providers,
            Route::Mcp => NavItem::Mcp,
            Route::Prompts => NavItem::Prompts,
            Route::Config | Route::ConfigWebDav => NavItem::Config,
            Route::Skills
            | Route::SkillsDiscover
            | Route::SkillsRepos
            | Route::SkillDetail { .. } => NavItem::Skills,
            Route::Settings | Route::SettingsProxy => NavItem::Settings,
        }
    }

    pub(crate) fn set_route_no_history(&mut self, route: Route) -> Action {
        if route == self.route {
            return Action::None;
        }

        self.route = route.clone();
        self.focus = route_default_focus(&route);

        let nav_item = Self::nav_item_for_route(&route);
        if let Some(idx) = NavItem::ALL.iter().position(|item| *item == nav_item) {
            self.nav_idx = idx;
        }

        if matches!(route, Route::Main) {
            self.route_stack.clear();
            self.focus = Focus::Nav;
        }

        Action::SwitchRoute(route)
    }

    pub(crate) fn push_route_and_switch(&mut self, route: Route) -> Action {
        if route == self.route {
            return Action::None;
        }
        self.route_stack.push(self.route.clone());
        self.set_route_no_history(route)
    }

    pub(crate) fn pop_route_and_switch(&mut self) -> Action {
        if let Some(prev) = self.route_stack.pop() {
            self.set_route_no_history(prev)
        } else {
            self.set_route_no_history(Route::Main)
        }
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some(toast) = &mut self.toast {
            if toast.remaining_ticks > 0 {
                toast.remaining_ticks -= 1;
            }
            if toast.remaining_ticks == 0 {
                self.toast = None;
            }
        }

        if let Some(transition) = self.proxy_visual_transition {
            if self.tick.saturating_sub(transition.started_tick) >= PROXY_HERO_TRANSITION_TICKS {
                self.proxy_visual_transition = None;
            }
        }
    }

    pub(crate) fn observe_proxy_visual_state(&mut self, data: &UiData) {
        let current_on = data.proxy.running;

        match self.proxy_visual_state.replace(current_on) {
            None => {}
            Some(previous_on) if previous_on != current_on => {
                self.proxy_visual_transition = Some(ProxyVisualTransition {
                    from_on: previous_on,
                    to_on: current_on,
                    started_tick: self.tick,
                });
            }
            Some(_) => {}
        }
    }

    pub(crate) fn should_poll_proxy_activity(&self) -> bool {
        matches!(self.route, Route::Main) && self.tick % PROXY_ACTIVITY_POLL_INTERVAL_TICKS == 0
    }

    pub(crate) fn reset_proxy_activity(&mut self, input_tokens: u64, output_tokens: u64) {
        self.proxy_input_activity_samples.clear();
        self.proxy_output_activity_samples.clear();
        self.proxy_activity_last_input_tokens = Some(input_tokens);
        self.proxy_activity_last_output_tokens = Some(output_tokens);
    }

    pub(crate) fn observe_proxy_token_activity(&mut self, input_tokens: u64, output_tokens: u64) {
        let Some(previous_input) = self.proxy_activity_last_input_tokens.replace(input_tokens)
        else {
            return;
        };
        let Some(previous_output) = self
            .proxy_activity_last_output_tokens
            .replace(output_tokens)
        else {
            return;
        };

        let (input_delta, output_delta) =
            if input_tokens < previous_input || output_tokens < previous_output {
                self.proxy_input_activity_samples.clear();
                self.proxy_output_activity_samples.clear();
                (0, 0)
            } else {
                (
                    input_tokens.saturating_sub(previous_input),
                    output_tokens.saturating_sub(previous_output),
                )
            };

        self.proxy_input_activity_samples.push(input_delta);
        self.proxy_output_activity_samples.push(output_delta);

        if self.proxy_input_activity_samples.len() > PROXY_ACTIVITY_WINDOW {
            let overflow = self.proxy_input_activity_samples.len() - PROXY_ACTIVITY_WINDOW;
            self.proxy_input_activity_samples.drain(0..overflow);
        }
        if self.proxy_output_activity_samples.len() > PROXY_ACTIVITY_WINDOW {
            let overflow = self.proxy_output_activity_samples.len() - PROXY_ACTIVITY_WINDOW;
            self.proxy_output_activity_samples.drain(0..overflow);
        }
    }

    pub fn push_toast(&mut self, message: impl Into<String>, kind: ToastKind) {
        self.toast = Some(Toast::new(message, kind));
    }

    pub fn open_help(&mut self) {
        self.overlay = Overlay::Help;
    }

    pub fn close_overlay(&mut self) {
        self.overlay = Overlay::None;
    }

    pub fn on_key(&mut self, key: KeyEvent, data: &UiData) -> Action {
        self.clamp_selections(data);

        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return Action::Quit;
        }

        if self.overlay.is_active() {
            return self.on_overlay_key(key, data);
        }

        if self.editor.is_some() {
            return self.on_editor_key(key);
        }

        if self.form.is_some() {
            return self.on_form_key(key, data);
        }

        if self.filter.active {
            return self.on_filter_key(key);
        }

        // Vim-style hjkl navigation
        let key = match key.code {
            KeyCode::Char('h') => KeyEvent::new(KeyCode::Left, key.modifiers),
            KeyCode::Char('j') => KeyEvent::new(KeyCode::Down, key.modifiers),
            KeyCode::Char('k') => KeyEvent::new(KeyCode::Up, key.modifiers),
            KeyCode::Char('l') => KeyEvent::new(KeyCode::Right, key.modifiers),
            _ => key,
        };

        // Global actions.
        match key.code {
            KeyCode::Char('?') => {
                self.open_help();
                return Action::None;
            }
            KeyCode::Char('/') => {
                self.filter.active = true;
                return Action::None;
            }
            KeyCode::Char('[') => return Action::SetAppType(cycle_app_type(&self.app_type, -1)),
            KeyCode::Char(']') => return Action::SetAppType(cycle_app_type(&self.app_type, 1)),
            KeyCode::Left => {
                self.focus = Focus::Nav;
                return Action::None;
            }
            KeyCode::Right => {
                if route_has_content_list(&self.route) {
                    self.focus = Focus::Content;
                } else {
                    self.focus = Focus::Nav;
                }
                return Action::None;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                return self.on_back_key();
            }
            _ => {}
        }

        if matches!(self.route, Route::Main)
            && matches!(key.code, KeyCode::Char('p') | KeyCode::Char('P'))
        {
            return self.main_proxy_action(data);
        }

        // Navigation + route-specific actions.
        match self.focus {
            Focus::Nav => self.on_nav_key(key),
            Focus::Content => self.on_content_key(key, data),
        }
    }

    pub(crate) fn on_back_key(&mut self) -> Action {
        match self.route {
            Route::Main => {
                self.overlay = Overlay::Confirm(ConfirmOverlay {
                    title: crate::cli::i18n::texts::tui_confirm_exit_title().to_string(),
                    message: crate::cli::i18n::texts::tui_confirm_exit_message().to_string(),
                    action: ConfirmAction::Quit,
                });
                Action::None
            }
            _ => self.pop_route_and_switch(),
        }
    }

    pub(crate) fn on_filter_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.filter.active = false;
                self.filter.buffer.clear();
            }
            KeyCode::Enter => {
                self.filter.active = false;
            }
            KeyCode::Backspace => {
                self.filter.buffer.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.filter.buffer.push(c);
                }
            }
            _ => {}
        }
        Action::None
    }

    pub(crate) fn on_nav_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => {
                self.nav_idx = self.nav_idx.saturating_sub(1);
                Action::None
            }
            KeyCode::Down => {
                self.nav_idx = (self.nav_idx + 1).min(NavItem::ALL.len() - 1);
                Action::None
            }
            KeyCode::Enter => {
                if let Some(route) = self.nav_item().to_route() {
                    self.push_route_and_switch(route)
                } else {
                    self.overlay = Overlay::Confirm(ConfirmOverlay {
                        title: crate::cli::i18n::texts::tui_confirm_exit_title().to_string(),
                        message: crate::cli::i18n::texts::tui_confirm_exit_message().to_string(),
                        action: ConfirmAction::Quit,
                    });
                    Action::None
                }
            }
            _ => Action::None,
        }
    }

    pub(crate) fn on_content_key(&mut self, key: KeyEvent, data: &UiData) -> Action {
        match self.route.clone() {
            Route::Providers => self.on_providers_key(key, data),
            Route::ProviderDetail { id } => self.on_provider_detail_key(key, data, &id),
            Route::Mcp => self.on_mcp_key(key, data),
            Route::Prompts => self.on_prompts_key(key, data),
            Route::Config => self.on_config_key(key, data),
            Route::ConfigWebDav => self.on_config_webdav_key(key, data),
            Route::Skills => self.on_skills_installed_key(key, data),
            Route::SkillsDiscover => self.on_skills_discover_key(key),
            Route::SkillsRepos => self.on_skills_repos_key(key, data),
            Route::SkillDetail { directory } => self.on_skill_detail_key(key, data, &directory),
            Route::Settings => self.on_settings_key(key, data),
            Route::SettingsProxy => self.on_settings_proxy_key(key, data),
            Route::Main => match key.code {
                KeyCode::Char('r') => Action::LocalEnvRefresh,
                KeyCode::Char('p') | KeyCode::Char('P') => self.main_proxy_action(data),
                _ => Action::None,
            },
        }
    }
    pub(crate) fn clamp_selections(&mut self, data: &UiData) {
        let providers_len = visible_providers(&self.filter, data).len();
        if providers_len == 0 {
            self.provider_idx = 0;
        } else {
            self.provider_idx = self.provider_idx.min(providers_len - 1);
        }

        let mcp_len = visible_mcp(&self.filter, data).len();
        if mcp_len == 0 {
            self.mcp_idx = 0;
        } else {
            self.mcp_idx = self.mcp_idx.min(mcp_len - 1);
        }

        let prompt_len = visible_prompts(&self.filter, data).len();
        if prompt_len == 0 {
            self.prompt_idx = 0;
        } else {
            self.prompt_idx = self.prompt_idx.min(prompt_len - 1);
        }

        let skills_len = visible_skills_installed(&self.filter, data).len();
        if skills_len == 0 {
            self.skills_idx = 0;
        } else {
            self.skills_idx = self.skills_idx.min(skills_len - 1);
        }

        let discover_len =
            visible_skills_discover(&self.filter, &self.skills_discover_results).len();
        if discover_len == 0 {
            self.skills_discover_idx = 0;
        } else {
            self.skills_discover_idx = self.skills_discover_idx.min(discover_len - 1);
        }

        let repos_len = visible_skills_repos(&self.filter, data).len();
        if repos_len == 0 {
            self.skills_repo_idx = 0;
        } else {
            self.skills_repo_idx = self.skills_repo_idx.min(repos_len - 1);
        }

        let unmanaged_len =
            visible_skills_unmanaged(&self.filter, &self.skills_unmanaged_results).len();
        if unmanaged_len == 0 {
            self.skills_unmanaged_idx = 0;
        } else {
            self.skills_unmanaged_idx = self.skills_unmanaged_idx.min(unmanaged_len - 1);
        }

        let config_len = visible_config_items(&self.filter).len();
        if config_len == 0 {
            self.config_idx = 0;
        } else {
            self.config_idx = self.config_idx.min(config_len - 1);
        }

        let config_webdav_len = visible_webdav_config_items(&self.filter).len();
        if config_webdav_len == 0 {
            self.config_webdav_idx = 0;
        } else {
            self.config_webdav_idx = self.config_webdav_idx.min(config_webdav_len - 1);
        }
    }
}
