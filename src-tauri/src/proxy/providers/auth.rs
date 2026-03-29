#[derive(Debug, Clone)]
pub struct AuthInfo {
    pub api_key: String,
    pub strategy: AuthStrategy,
    pub access_token: Option<String>,
}

impl AuthInfo {
    pub fn new(api_key: String, strategy: AuthStrategy) -> Self {
        Self {
            api_key,
            strategy,
            access_token: None,
        }
    }

    pub fn with_access_token(api_key: String, access_token: String) -> Self {
        Self {
            api_key,
            strategy: AuthStrategy::GoogleOAuth,
            access_token: Some(access_token),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStrategy {
    Anthropic,
    ClaudeAuth,
    Bearer,
    Google,
    GoogleOAuth,
    GitHubCopilot,
}
