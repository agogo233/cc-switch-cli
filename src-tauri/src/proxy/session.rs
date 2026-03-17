use axum::http::HeaderMap;
use serde_json::Value;

pub fn extract_session_id(headers: &HeaderMap, body: &Value, client_format: &str) -> String {
    if matches!(client_format, "codex" | "openai") {
        for header_name in ["session_id", "x-session-id"] {
            if let Some(session_id) = headers.get(header_name).and_then(|v| v.to_str().ok()) {
                if !session_id.is_empty() {
                    return format!("codex_{session_id}");
                }
            }
        }

        if let Some(previous_response_id) =
            body.get("previous_response_id").and_then(|v| v.as_str())
        {
            if !previous_response_id.is_empty() {
                return format!("codex_{previous_response_id}");
            }
        }
    }

    if let Some(user_id) = body
        .get("metadata")
        .and_then(|metadata| metadata.get("user_id"))
        .and_then(|v| v.as_str())
    {
        if let Some((_, session_id)) = user_id.split_once("_session_") {
            if !session_id.is_empty() {
                return session_id.to_string();
            }
        }
    }

    if let Some(session_id) = body
        .get("metadata")
        .and_then(|metadata| metadata.get("session_id"))
        .and_then(|v| v.as_str())
    {
        if !session_id.is_empty() {
            return session_id.to_string();
        }
    }

    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_claude_metadata_session_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "metadata": {
                "session_id": "claude-session-123"
            }
        });

        assert_eq!(
            extract_session_id(&headers, &body, "claude"),
            "claude-session-123"
        );
    }

    #[test]
    fn extracts_codex_previous_response_id() {
        let headers = HeaderMap::new();
        let body = json!({
            "previous_response_id": "resp_abc123"
        });

        assert_eq!(
            extract_session_id(&headers, &body, "codex"),
            "codex_resp_abc123"
        );
    }
}
