use bytes::Bytes;
use serde_json::Value;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ParsedUsage {
    pub model: String,
    pub usage: TokenUsage,
}

pub fn parse_claude_response_usage(body: &[u8]) -> Option<ParsedUsage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    parse_claude_response_value(&value)
}

pub fn fallback_model_from_response_bytes(body: &[u8], request_model: &str) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| response_model(&value))
        .unwrap_or_else(|| request_model.to_string())
}

pub fn error_message_from_response_bytes(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    error_message_from_value(&value)
}

pub fn parse_claude_stream_usage(events: &[Value]) -> Option<ParsedUsage> {
    let mut usage = TokenUsage::default();
    let mut model = None;

    for event in events {
        match event.get("type").and_then(|value| value.as_str()) {
            Some("message_start") => {
                if model.is_none() {
                    model = event
                        .get("message")
                        .and_then(|message| message.get("model"))
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string());
                }

                if let Some(message_usage) = event
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    if let Some(input_tokens) = message_usage
                        .get("input_tokens")
                        .and_then(|value| value.as_u64())
                    {
                        usage.input_tokens = input_tokens as u32;
                    }
                    usage.cache_read_tokens = message_usage
                        .get("cache_read_input_tokens")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0) as u32;
                    usage.cache_creation_tokens = message_usage
                        .get("cache_creation_input_tokens")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0) as u32;
                }
            }
            Some("message_delta") => {
                if let Some(delta_usage) = event.get("usage") {
                    if let Some(output_tokens) = delta_usage
                        .get("output_tokens")
                        .and_then(|value| value.as_u64())
                    {
                        usage.output_tokens = output_tokens as u32;
                    }
                    if usage.input_tokens == 0 {
                        if let Some(input_tokens) = delta_usage
                            .get("input_tokens")
                            .and_then(|value| value.as_u64())
                        {
                            usage.input_tokens = input_tokens as u32;
                        }
                    }
                    if usage.cache_read_tokens == 0 {
                        if let Some(cache_read_tokens) = delta_usage
                            .get("cache_read_input_tokens")
                            .and_then(|value| value.as_u64())
                        {
                            usage.cache_read_tokens = cache_read_tokens as u32;
                        }
                    }
                    if usage.cache_creation_tokens == 0 {
                        if let Some(cache_creation_tokens) = delta_usage
                            .get("cache_creation_input_tokens")
                            .and_then(|value| value.as_u64())
                        {
                            usage.cache_creation_tokens = cache_creation_tokens as u32;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if usage.input_tokens == 0 && usage.output_tokens == 0 {
        return None;
    }

    Some(ParsedUsage {
        model: model.unwrap_or_default(),
        usage,
    })
}

pub fn fallback_model_from_stream_events(events: &[Value], request_model: &str) -> String {
    events
        .iter()
        .find_map(|event| {
            event
                .get("message")
                .and_then(|message| message.get("model"))
                .and_then(|value| value.as_str())
        })
        .map(|value| value.to_string())
        .unwrap_or_else(|| request_model.to_string())
}

pub fn error_message_from_stream_events(events: &[Value]) -> Option<String> {
    events.iter().find_map(error_message_from_value)
}

#[derive(Clone)]
pub struct StreamLogCollector {
    buffer: String,
    events: Vec<Value>,
    started_at: Instant,
    first_event_ms: Option<u64>,
}

impl StreamLogCollector {
    pub fn new(started_at: Instant) -> Self {
        Self {
            buffer: String::new(),
            events: Vec::new(),
            started_at,
            first_event_ms: None,
        }
    }

    pub fn record_chunk(&mut self, chunk: &Bytes) {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));

        while let Some((pos, delimiter_len)) = next_sse_block_boundary(&self.buffer) {
            let block = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + delimiter_len..].to_string();

            if block.trim().is_empty() {
                continue;
            }

            let mut data_lines = Vec::new();
            for line in block.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data
                        .strip_prefix(' ')
                        .unwrap_or(data)
                        .trim_end_matches('\r');
                    if data.trim() == "[DONE]" {
                        continue;
                    }
                    data_lines.push(data);
                }
            }

            if data_lines.is_empty() {
                continue;
            }

            let payload = data_lines.join("\n");
            if let Ok(event) = serde_json::from_str::<Value>(&payload) {
                if self.first_event_ms.is_none() {
                    self.first_event_ms = Some(self.started_at.elapsed().as_millis() as u64);
                }
                self.events.push(event);
            }
        }
    }

    pub fn parsed_usage(&self) -> Option<ParsedUsage> {
        parse_claude_stream_usage(&self.events)
    }

    pub fn fallback_model(&self, request_model: &str) -> String {
        fallback_model_from_stream_events(&self.events, request_model)
    }

    pub fn first_event_ms(&self) -> Option<u64> {
        self.first_event_ms
    }

    pub fn error_message(&self) -> Option<String> {
        error_message_from_stream_events(&self.events)
    }
}

fn next_sse_block_boundary(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|pos| (pos, 2));
    let crlf = buffer.find("\r\n\r\n").map(|pos| (pos, 4));

    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn parse_claude_response_value(body: &Value) -> Option<ParsedUsage> {
    let usage = body.get("usage")?;
    let parsed = TokenUsage {
        input_tokens: usage.get("input_tokens")?.as_u64()? as u32,
        output_tokens: usage.get("output_tokens")?.as_u64()? as u32,
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32,
        cache_creation_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32,
    };

    Some(ParsedUsage {
        model: response_model(body).unwrap_or_default(),
        usage: parsed,
    })
}

fn response_model(body: &Value) -> Option<String> {
    body.get("model")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn error_message_from_value(body: &Value) -> Option<String> {
    [
        body.pointer("/error/message"),
        body.pointer("/message"),
        body.pointer("/detail"),
        body.pointer("/error"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| value.as_str().map(ToString::to_string))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn stream_log_collector_accepts_crlf_and_data_without_space() {
        let mut collector = StreamLogCollector::new(Instant::now());

        collector.record_chunk(&Bytes::from_static(
            b"data:{\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-5-haiku\",\"usage\":{\"input_tokens\":11}}}\r\n\r\ndata:{\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\r\n\r\ndata:{\"type\":\"error\",\"error\":{\"message\":\"quota exceeded\"}}\r\n\r\ndata:[DONE]\r\n\r\n",
        ));

        let parsed = collector.parsed_usage().expect("parse usage");
        assert_eq!(parsed.model, "claude-3-5-haiku");
        assert_eq!(parsed.usage.input_tokens, 11);
        assert_eq!(parsed.usage.output_tokens, 7);
        assert_eq!(collector.error_message().as_deref(), Some("quota exceeded"));
        assert!(collector.first_event_ms().is_some());
    }

    #[test]
    fn stream_log_collector_collects_split_sse_event_across_chunks() {
        let mut collector = StreamLogCollector::new(Instant::now());

        collector.record_chunk(&Bytes::from_static(
            b"data:{\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-5-haiku\",\"usage\":{\"input_tokens\":11}}}\r\n\r\ndata:{\"type\":\"message_delta\",\"usage\":{\"out",
        ));

        let partial = collector.parsed_usage().expect("parse partial usage");
        assert_eq!(partial.model, "claude-3-5-haiku");
        assert_eq!(partial.usage.input_tokens, 11);
        assert_eq!(partial.usage.output_tokens, 0);

        collector.record_chunk(&Bytes::from_static(b"put_tokens\":7}}\r\n\r\n"));

        let parsed = collector.parsed_usage().expect("parse completed usage");
        assert_eq!(parsed.model, "claude-3-5-haiku");
        assert_eq!(parsed.usage.input_tokens, 11);
        assert_eq!(parsed.usage.output_tokens, 7);
        assert!(collector.first_event_ms().is_some());
    }

    #[test]
    fn stream_log_collector_collects_event_when_crlf_delimiter_is_split_across_chunks() {
        let mut collector = StreamLogCollector::new(Instant::now());

        collector.record_chunk(&Bytes::from_static(
            b"data:{\"type\":\"message_start\",\"message\":{\"model\":\"claude-3-5-haiku\",\"usage\":{\"input_tokens\":11}}}\r\n",
        ));

        assert!(collector.parsed_usage().is_none());
        assert!(collector.first_event_ms().is_none());

        collector.record_chunk(&Bytes::from_static(
            b"\r\ndata:{\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\r\n\r\n",
        ));

        let parsed = collector
            .parsed_usage()
            .expect("parse usage after split delimiter");
        assert_eq!(parsed.model, "claude-3-5-haiku");
        assert_eq!(parsed.usage.input_tokens, 11);
        assert_eq!(parsed.usage.output_tokens, 7);
        assert!(collector.first_event_ms().is_some());
    }
}
