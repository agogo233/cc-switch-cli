use super::CodexWireApi;

#[derive(Debug, Default)]
pub(crate) struct ParsedCodexConfigSnippet {
    pub(crate) base_url: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) wire_api: Option<CodexWireApi>,
    pub(crate) requires_openai_auth: Option<bool>,
    pub(crate) env_key: Option<String>,
}

pub(crate) fn parse_codex_config_snippet(cfg: &str) -> ParsedCodexConfigSnippet {
    let mut out = ParsedCodexConfigSnippet::default();
    let table: toml::Table = match toml::from_str(cfg.trim()) {
        Ok(table) => table,
        Err(_) => return out,
    };

    out.model = table
        .get("model")
        .and_then(|value| value.as_str())
        .map(String::from);

    let section = table
        .get("model_provider")
        .and_then(|value| value.as_str())
        .and_then(|key| {
            table
                .get("model_providers")
                .and_then(|value| value.as_table())
                .and_then(|providers| providers.get(key))
                .and_then(|value| value.as_table())
        });

    if let Some(section) = section {
        out.base_url = section
            .get("base_url")
            .and_then(|value| value.as_str())
            .map(String::from);
        out.wire_api = section
            .get("wire_api")
            .and_then(|value| value.as_str())
            .and_then(|value| match value {
                "chat" => Some(CodexWireApi::Chat),
                "responses" => Some(CodexWireApi::Responses),
                _ => None,
            });
        out.requires_openai_auth = section
            .get("requires_openai_auth")
            .and_then(|value| value.as_bool());
        out.env_key = section
            .get("env_key")
            .and_then(|value| value.as_str())
            .map(String::from);
    }

    out
}

pub(crate) fn update_codex_config_snippet(
    original: &str,
    base_url: &str,
    model: &str,
    wire_api: CodexWireApi,
    requires_openai_auth: bool,
    env_key: &str,
) -> String {
    let mut doc = match original.trim().parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return original.to_string(),
    };

    if let Some(model) = non_empty(model) {
        doc["model"] = toml_edit::value(model);
    } else {
        doc.remove("model");
    }

    let provider_key = doc
        .get("model_provider")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    if let Some(key) = provider_key {
        if doc.get("model_providers").is_none() {
            doc["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let providers = doc["model_providers"]
            .as_table_like_mut()
            .expect("model_providers should be a table");
        if providers.get(&key).is_none() {
            providers.insert(&key, toml_edit::Item::Table(toml_edit::Table::new()));
        }

        if let Some(section) = providers
            .get_mut(&key)
            .and_then(|value| value.as_table_like_mut())
        {
            if let Some(base_url) = non_empty(base_url) {
                section.insert("base_url", toml_edit::value(base_url));
            } else {
                section.remove("base_url");
            }

            section.insert("wire_api", toml_edit::value(wire_api.as_str()));
            section.insert(
                "requires_openai_auth",
                toml_edit::value(requires_openai_auth),
            );

            if requires_openai_auth {
                section.remove("env_key");
            } else {
                let env_key = non_empty(env_key).unwrap_or("OPENAI_API_KEY");
                section.insert("env_key", toml_edit::value(env_key));
            }
        }
    }

    let result = doc.to_string();
    let trimmed = result.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn clean_codex_provider_key(provider_id: &str, provider_name: &str) -> String {
    let raw = if provider_id.trim().is_empty() {
        provider_name.trim()
    } else {
        provider_id.trim()
    };
    crate::codex_config::clean_codex_provider_key(raw)
}

pub(crate) fn build_codex_provider_config_toml(
    provider_key: &str,
    base_url: &str,
    model: &str,
    wire_api: CodexWireApi,
) -> String {
    let provider_key = escape_toml_string(provider_key);
    let model = escape_toml_string(model);
    let base_url = escape_toml_string(base_url);

    [
        format!("model_provider = \"{}\"", provider_key),
        format!("model = \"{}\"", model),
        "model_reasoning_effort = \"high\"".to_string(),
        "disable_response_storage = true".to_string(),
        String::new(),
        format!("[model_providers.{}]", provider_key),
        format!("name = \"{}\"", provider_key),
        format!("base_url = \"{}\"", base_url),
        format!("wire_api = \"{}\"", wire_api.as_str()),
        "requires_openai_auth = true".to_string(),
        String::new(),
    ]
    .join("\n")
}

pub(crate) fn merge_codex_common_config_snippet(
    config_toml: &str,
    common_snippet: &str,
) -> Result<String, String> {
    use toml_edit::DocumentMut;

    let common_trimmed = common_snippet.trim();
    if common_trimmed.is_empty() {
        return Ok(config_toml.to_string());
    }

    let mut common_doc: DocumentMut = common_trimmed
        .parse()
        .map_err(|e| format!("Invalid common Codex TOML: {e}"))?;

    let config_trimmed = config_toml.trim();
    let config_doc: DocumentMut = if config_trimmed.is_empty() {
        DocumentMut::default()
    } else {
        config_trimmed
            .parse()
            .map_err(|e| format!("Invalid provider Codex TOML: {e}"))?
    };

    merge_toml_tables(common_doc.as_table_mut(), config_doc.as_table());
    Ok(common_doc.to_string())
}

pub(crate) fn strip_codex_common_config_snippet(
    config_toml: &str,
    common_snippet: &str,
) -> Result<String, String> {
    use toml_edit::DocumentMut;

    let common_trimmed = common_snippet.trim();
    if common_trimmed.is_empty() {
        return Ok(config_toml.to_string());
    }

    let common_doc: DocumentMut = common_trimmed
        .parse()
        .map_err(|e| format!("Invalid common Codex TOML: {e}"))?;

    let config_trimmed = config_toml.trim();
    if config_trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut config_doc: DocumentMut = config_trimmed
        .parse()
        .map_err(|e| format!("Invalid provider Codex TOML: {e}"))?;
    strip_toml_tables(config_doc.as_table_mut(), common_doc.as_table());
    Ok(config_doc.to_string())
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn escape_toml_string(value: &str) -> String {
    value.replace('"', "\\\"")
}

fn merge_toml_tables(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    for (key, src_item) in src.iter() {
        match (dst.get_mut(key), src_item) {
            (Some(dst_item), toml_edit::Item::Table(src_table)) if dst_item.is_table() => {
                if let Some(dst_table) = dst_item.as_table_mut() {
                    merge_toml_tables(dst_table, src_table);
                }
            }
            _ => {
                dst.insert(key, src_item.clone());
            }
        }
    }
}

fn strip_toml_tables(dst: &mut toml_edit::Table, common: &toml_edit::Table) {
    let mut keys_to_remove = Vec::new();

    for (key, common_item) in common.iter() {
        let Some(dst_item) = dst.get_mut(key) else {
            continue;
        };

        match (dst_item, common_item) {
            (toml_edit::Item::Table(dst_table), toml_edit::Item::Table(common_table)) => {
                strip_toml_tables(dst_table, common_table);
                if dst_table.is_empty() {
                    keys_to_remove.push(key.to_string());
                }
            }
            (dst_item, common_item) => {
                if toml_items_equal(dst_item, common_item) {
                    keys_to_remove.push(key.to_string());
                }
            }
        }
    }

    for key in keys_to_remove {
        dst.remove(&key);
    }
}

fn toml_items_equal(left: &toml_edit::Item, right: &toml_edit::Item) -> bool {
    match (left.as_value(), right.as_value()) {
        (Some(left_value), Some(right_value)) => {
            left_value.to_string().trim() == right_value.to_string().trim()
        }
        _ => left.to_string().trim() == right.to_string().trim(),
    }
}
