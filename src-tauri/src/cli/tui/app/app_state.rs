use super::*;

#[derive(Debug, Clone)]
pub enum Action {
    None,
    ReloadData,
    SwitchRoute(Route),
    Quit,
    SetAppType(AppType),
    LocalEnvRefresh,

    SkillsToggle {
        directory: String,
        enabled: bool,
    },
    SkillsSetApps {
        directory: String,
        apps: crate::app_config::SkillApps,
    },
    SkillsInstall {
        spec: String,
    },
    SkillsUninstall {
        directory: String,
    },
    SkillsSync {
        app: Option<AppType>,
    },
    SkillsSetSyncMethod {
        method: SyncMethod,
    },
    SkillsDiscover {
        query: String,
    },
    SkillsRepoAdd {
        spec: String,
    },
    SkillsRepoRemove {
        owner: String,
        name: String,
    },
    SkillsRepoToggleEnabled {
        owner: String,
        name: String,
        enabled: bool,
    },
    SkillsOpenImport,
    SkillsScanUnmanaged,
    SkillsImportFromApps {
        directories: Vec<String>,
    },

    ProviderSwitch {
        id: String,
    },
    ProviderDelete {
        id: String,
    },
    ProviderSpeedtest {
        url: String,
    },
    ProviderStreamCheck {
        id: String,
    },
    ProviderModelFetch {
        base_url: String,
        api_key: Option<String>,
        field: ProviderAddField,
        claude_idx: Option<usize>,
    },

    McpToggle {
        id: String,
        enabled: bool,
    },
    McpSetApps {
        id: String,
        apps: crate::app_config::McpApps,
    },
    McpDelete {
        id: String,
    },
    McpImport,

    PromptActivate {
        id: String,
    },
    PromptDeactivate {
        id: String,
    },
    PromptDelete {
        id: String,
    },

    ConfigExport {
        path: String,
    },
    ConfigImport {
        path: String,
    },
    ConfigBackup {
        name: Option<String>,
    },
    ConfigRestoreBackup {
        id: String,
    },
    ConfigShowFull,
    ConfigValidate,
    ConfigOpenProxyHelp,
    ConfigCommonSnippetClear {
        app_type: AppType,
    },
    ConfigCommonSnippetApply {
        app_type: AppType,
    },
    ConfigWebDavCheckConnection,
    ConfigWebDavUpload,
    ConfigWebDavDownload,
    ConfigWebDavMigrateV1ToV2,
    ConfigWebDavReset,
    ConfigWebDavJianguoyunQuickSetup {
        username: String,
        password: String,
    },
    ConfigReset,

    EditorSubmit {
        submit: EditorSubmit,
        content: String,
    },
    EditorDiscard,
    EditorOpenExternal,

    SetSkipClaudeOnboarding {
        enabled: bool,
    },
    SetClaudePluginIntegration {
        enabled: bool,
    },
    SetProxyEnabled {
        enabled: bool,
    },
    SetProxyTakeover {
        app_type: AppType,
        enabled: bool,
    },
    SetManagedProxyForCurrentApp {
        app_type: AppType,
        enabled: bool,
    },
    SetLanguage(Language),

    CheckUpdate,
    ConfirmUpdate,
    CancelUpdate,
    CancelUpdateCheck,
}

#[derive(Debug, Clone)]
pub enum ConfigItem {
    Path,
    ShowFull,
    Export,
    Import,
    Backup,
    Restore,
    Validate,
    CommonSnippet,
    Proxy,
    WebDavSync,
    Reset,
}

impl ConfigItem {
    pub const ALL: [ConfigItem; 10] = [
        ConfigItem::Path,
        ConfigItem::ShowFull,
        ConfigItem::Export,
        ConfigItem::Import,
        ConfigItem::Backup,
        ConfigItem::Restore,
        ConfigItem::Validate,
        ConfigItem::CommonSnippet,
        ConfigItem::WebDavSync,
        ConfigItem::Reset,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsItem {
    Language,
    SkipClaudeOnboarding,
    ClaudePluginIntegration,
    Proxy,
    CheckForUpdates,
}

impl SettingsItem {
    pub const ALL: [SettingsItem; 4] = [
        SettingsItem::Language,
        SettingsItem::SkipClaudeOnboarding,
        SettingsItem::ClaudePluginIntegration,
        SettingsItem::CheckForUpdates,
    ];
}

#[derive(Debug, Clone)]
pub enum WebDavConfigItem {
    Settings,
    CheckConnection,
    Upload,
    Download,
    Reset,
    JianguoyunQuickSetup,
}

impl WebDavConfigItem {
    pub const ALL: [WebDavConfigItem; 6] = [
        WebDavConfigItem::Settings,
        WebDavConfigItem::CheckConnection,
        WebDavConfigItem::Upload,
        WebDavConfigItem::Download,
        WebDavConfigItem::Reset,
        WebDavConfigItem::JianguoyunQuickSetup,
    ];
}

pub(crate) const PROXY_HERO_TRANSITION_TICKS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyVisualTransition {
    pub from_on: bool,
    pub to_on: bool,
    pub started_tick: u64,
}

#[derive(Debug, Clone)]
pub struct App {
    pub app_type: AppType,
    pub route: Route,
    pub route_stack: Vec<Route>,
    pub focus: Focus,
    pub nav_idx: usize,

    pub filter: FilterState,
    pub editor: Option<EditorState>,
    pub form: Option<FormState>,
    pub overlay: Overlay,
    pub toast: Option<Toast>,
    pub should_quit: bool,
    pub last_size: Size,
    pub tick: u64,
    pub proxy_activity_samples: Vec<u64>,
    pub proxy_activity_last_total_requests: Option<u64>,
    pub proxy_visual_state: Option<bool>,
    pub proxy_visual_transition: Option<ProxyVisualTransition>,

    pub local_env_results: Vec<crate::services::local_env_check::ToolCheckResult>,
    pub local_env_loading: bool,

    pub provider_idx: usize,
    pub mcp_idx: usize,
    pub prompt_idx: usize,
    pub skills_idx: usize,
    pub skills_discover_idx: usize,
    pub skills_repo_idx: usize,
    pub skills_unmanaged_idx: usize,
    pub skills_discover_results: Vec<crate::services::skill::Skill>,
    pub skills_discover_query: String,
    pub skills_unmanaged_results: Vec<crate::services::skill::UnmanagedSkill>,
    pub skills_unmanaged_selected: HashSet<String>,
    pub config_idx: usize,
    pub config_webdav_idx: usize,
    pub webdav_quick_setup_username: Option<String>,
    pub language_idx: usize,
    pub settings_idx: usize,
}
