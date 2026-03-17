use crate::provider::Provider;
use serde_json::Value;

pub struct ModelMapping {
    pub haiku_model: Option<String>,
    pub sonnet_model: Option<String>,
    pub opus_model: Option<String>,
    pub default_model: Option<String>,
    pub reasoning_model: Option<String>,
}

impl ModelMapping {
    pub fn from_provider(provider: &Provider) -> Self {
        let env = provider.settings_config.get("env");

        Self {
            haiku_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_HAIKU_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            sonnet_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_SONNET_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            opus_model: env
                .and_then(|value| value.get("ANTHROPIC_DEFAULT_OPUS_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            default_model: env
                .and_then(|value| value.get("ANTHROPIC_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
            reasoning_model: env
                .and_then(|value| value.get("ANTHROPIC_REASONING_MODEL"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(String::from),
        }
    }

    pub fn has_mapping(&self) -> bool {
        self.haiku_model.is_some()
            || self.sonnet_model.is_some()
            || self.opus_model.is_some()
            || self.default_model.is_some()
            || self.reasoning_model.is_some()
    }

    pub fn map_model(&self, original_model: &str, has_thinking: bool) -> String {
        let model_lower = original_model.to_lowercase();

        if has_thinking {
            if let Some(model) = &self.reasoning_model {
                return model.clone();
            }
        }

        if model_lower.contains("haiku") {
            if let Some(model) = &self.haiku_model {
                return model.clone();
            }
        }
        if model_lower.contains("opus") {
            if let Some(model) = &self.opus_model {
                return model.clone();
            }
        }
        if model_lower.contains("sonnet") {
            if let Some(model) = &self.sonnet_model {
                return model.clone();
            }
        }

        if let Some(model) = &self.default_model {
            return model.clone();
        }

        original_model.to_string()
    }
}

pub fn has_thinking_enabled(body: &Value) -> bool {
    match body
        .get("thinking")
        .and_then(Value::as_object)
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
    {
        Some("enabled") | Some("adaptive") => true,
        Some("disabled") | None => false,
        Some(other) => {
            log::warn!(
                "[ModelMapper] unknown thinking.type='{other}', treat as disabled to avoid misrouting reasoning model"
            );
            false
        }
    }
}

pub fn apply_model_mapping(
    mut body: Value,
    provider: &Provider,
) -> (Value, Option<String>, Option<String>) {
    let mapping = ModelMapping::from_provider(provider);

    if !mapping.has_mapping() {
        let original = body.get("model").and_then(Value::as_str).map(String::from);
        return (body, original, None);
    }

    let original_model = body.get("model").and_then(Value::as_str).map(String::from);

    if let Some(original) = &original_model {
        let mapped = mapping.map_model(original, has_thinking_enabled(&body));

        if mapped != *original {
            body["model"] = serde_json::json!(mapped);
            return (body, Some(original.clone()), Some(mapped));
        }
    }

    (body, original_model, None)
}
