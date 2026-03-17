use serde_json::Value;
use std::collections::HashSet;

#[cfg(test)]
pub fn filter_private_params(body: Value) -> Value {
    filter_private_params_with_whitelist(body, &[])
}

pub fn filter_private_params_with_whitelist(body: Value, whitelist: &[String]) -> Value {
    let whitelist_set: HashSet<&str> = whitelist.iter().map(|item| item.as_str()).collect();
    filter_recursive_with_whitelist(body, &mut Vec::new(), &whitelist_set)
}

fn filter_recursive_with_whitelist(
    value: Value,
    removed_keys: &mut Vec<String>,
    whitelist: &HashSet<&str>,
) -> Value {
    match value {
        Value::Object(map) => {
            let filtered = map
                .into_iter()
                .filter_map(|(key, value)| {
                    if key.starts_with('_') && !whitelist.contains(key.as_str()) {
                        removed_keys.push(key);
                        None
                    } else {
                        Some((
                            key,
                            filter_recursive_with_whitelist(value, removed_keys, whitelist),
                        ))
                    }
                })
                .collect();

            if !removed_keys.is_empty() {
                log::debug!("[BodyFilter] filtered private params: {removed_keys:?}");
                removed_keys.clear();
            }

            Value::Object(filtered)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| filter_recursive_with_whitelist(value, removed_keys, whitelist))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filters_private_params_recursively() {
        let input = json!({
            "model": "claude-3-5-sonnet",
            "_top_secret": true,
            "messages": [{
                "role": "user",
                "_message_secret": true,
                "content": [{
                    "type": "text",
                    "text": "hello",
                    "_content_secret": true
                }]
            }],
            "metadata": {
                "keep": "ok",
                "_trace_id": "drop"
            }
        });

        let output = filter_private_params(input);

        assert!(output.get("_top_secret").is_none());
        assert!(output.pointer("/messages/0/_message_secret").is_none());
        assert!(output
            .pointer("/messages/0/content/0/_content_secret")
            .is_none());
        assert!(output.pointer("/metadata/_trace_id").is_none());
        assert_eq!(
            output
                .pointer("/metadata/keep")
                .and_then(|value| value.as_str()),
            Some("ok")
        );
    }
}
