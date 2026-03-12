use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::proxy::response::StreamCompletion;

#[derive(Debug, Deserialize)]
struct OpenAIStreamChunk {
    id: String,
    model: String,
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DeltaToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "type", default)]
    call_type: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

pub fn create_anthropic_sse_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    stream_completion: StreamCompletion,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut message_id = None;
        let mut current_model = None;
        let mut content_index = 0;
        let mut has_sent_message_start = false;
        let mut current_block_type: Option<String> = None;
        let mut tool_call_id = None;

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    while let Some(pos) = buffer.find("\n\n") {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        if line.trim().is_empty() {
                            continue;
                        }

                        for raw_line in line.lines() {
                            let Some(data) = raw_line.strip_prefix("data: ") else {
                                continue;
                            };

                            if data.trim() == "[DONE]" {
                                let event = json!({"type": "message_stop"});
                                let sse_data = format!(
                                    "event: message_stop\ndata: {}\n\n",
                                    serde_json::to_string(&event).unwrap_or_default()
                                );
                                yield Ok(Bytes::from(sse_data));
                                continue;
                            }

                            let Ok(chunk) = serde_json::from_str::<OpenAIStreamChunk>(data) else {
                                continue;
                            };

                            if message_id.is_none() {
                                message_id = Some(chunk.id.clone());
                            }
                            if current_model.is_none() {
                                current_model = Some(chunk.model.clone());
                            }

                            let Some(choice) = chunk.choices.first() else {
                                continue;
                            };

                            if !has_sent_message_start {
                                let event = json!({
                                    "type": "message_start",
                                    "message": {
                                        "id": message_id.clone().unwrap_or_default(),
                                        "type": "message",
                                        "role": "assistant",
                                        "model": current_model.clone().unwrap_or_default(),
                                        "usage": {
                                            "input_tokens": 0,
                                            "output_tokens": 0
                                        }
                                    }
                                });
                                let sse_data = format!(
                                    "event: message_start\ndata: {}\n\n",
                                    serde_json::to_string(&event).unwrap_or_default()
                                );
                                yield Ok(Bytes::from(sse_data));
                                has_sent_message_start = true;
                            }

                            if let Some(reasoning) = &choice.delta.reasoning {
                                if current_block_type.is_none() {
                                    let event = json!({
                                        "type": "content_block_start",
                                        "index": content_index,
                                        "content_block": {
                                            "type": "thinking",
                                            "thinking": ""
                                        }
                                    });
                                    let sse_data = format!(
                                        "event: content_block_start\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default()
                                    );
                                    yield Ok(Bytes::from(sse_data));
                                    current_block_type = Some("thinking".to_string());
                                }

                                let event = json!({
                                    "type": "content_block_delta",
                                    "index": content_index,
                                    "delta": {
                                        "type": "thinking_delta",
                                        "thinking": reasoning
                                    }
                                });
                                let sse_data = format!(
                                    "event: content_block_delta\ndata: {}\n\n",
                                    serde_json::to_string(&event).unwrap_or_default()
                                );
                                yield Ok(Bytes::from(sse_data));
                            }

                            if let Some(content) = &choice.delta.content {
                                if current_block_type.as_deref() != Some("text") {
                                    if current_block_type.is_some() {
                                        let event = json!({
                                            "type": "content_block_stop",
                                            "index": content_index
                                        });
                                        let sse_data = format!(
                                            "event: content_block_stop\ndata: {}\n\n",
                                            serde_json::to_string(&event).unwrap_or_default()
                                        );
                                        yield Ok(Bytes::from(sse_data));
                                        content_index += 1;
                                    }

                                    let event = json!({
                                        "type": "content_block_start",
                                        "index": content_index,
                                        "content_block": {
                                            "type": "text",
                                            "text": ""
                                        }
                                    });
                                    let sse_data = format!(
                                        "event: content_block_start\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default()
                                    );
                                    yield Ok(Bytes::from(sse_data));
                                    current_block_type = Some("text".to_string());
                                }

                                let event = json!({
                                    "type": "content_block_delta",
                                    "index": content_index,
                                    "delta": {
                                        "type": "text_delta",
                                        "text": content
                                    }
                                });
                                let sse_data = format!(
                                    "event: content_block_delta\ndata: {}\n\n",
                                    serde_json::to_string(&event).unwrap_or_default()
                                );
                                yield Ok(Bytes::from(sse_data));
                            }

                            if let Some(tool_calls) = &choice.delta.tool_calls {
                                for tool_call in tool_calls {
                                    if let Some(id) = &tool_call.id {
                                        if current_block_type.is_some() {
                                            let event = json!({
                                                "type": "content_block_stop",
                                                "index": content_index
                                            });
                                            let sse_data = format!(
                                                "event: content_block_stop\ndata: {}\n\n",
                                                serde_json::to_string(&event).unwrap_or_default()
                                            );
                                            yield Ok(Bytes::from(sse_data));
                                            content_index += 1;
                                        }

                                        tool_call_id = Some(id.clone());
                                    }

                                    if let Some(function) = &tool_call.function {
                                        if let Some(name) = &function.name {
                                            let event = json!({
                                                "type": "content_block_start",
                                                "index": content_index,
                                                "content_block": {
                                                    "type": "tool_use",
                                                    "id": tool_call_id.clone().unwrap_or_default(),
                                                    "name": name
                                                }
                                            });
                                            let sse_data = format!(
                                                "event: content_block_start\ndata: {}\n\n",
                                                serde_json::to_string(&event).unwrap_or_default()
                                            );
                                            yield Ok(Bytes::from(sse_data));
                                            current_block_type = Some("tool_use".to_string());
                                        }

                                        if let Some(args) = &function.arguments {
                                            let event = json!({
                                                "type": "content_block_delta",
                                                "index": content_index,
                                                "delta": {
                                                    "type": "input_json_delta",
                                                    "partial_json": args
                                                }
                                            });
                                            let sse_data = format!(
                                                "event: content_block_delta\ndata: {}\n\n",
                                                serde_json::to_string(&event).unwrap_or_default()
                                            );
                                            yield Ok(Bytes::from(sse_data));
                                        }
                                    }
                                }
                            }

                            if let Some(finish_reason) = &choice.finish_reason {
                                if current_block_type.is_some() {
                                    let event = json!({
                                        "type": "content_block_stop",
                                        "index": content_index
                                    });
                                    let sse_data = format!(
                                        "event: content_block_stop\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default()
                                    );
                                    yield Ok(Bytes::from(sse_data));
                                }

                                let usage_json = chunk.usage.as_ref().map(|usage| json!({
                                    "input_tokens": usage.prompt_tokens,
                                    "output_tokens": usage.completion_tokens
                                }));
                                let event = json!({
                                    "type": "message_delta",
                                    "delta": {
                                        "stop_reason": map_stop_reason(Some(finish_reason)),
                                        "stop_sequence": null
                                    },
                                    "usage": usage_json
                                });
                                let sse_data = format!(
                                    "event: message_delta\ndata: {}\n\n",
                                    serde_json::to_string(&event).unwrap_or_default()
                                );
                                yield Ok(Bytes::from(sse_data));
                            }
                        }
                    }
                }
                Err(error) => {
                    stream_completion.record_error(error.to_string());
                    let error_event = json!({
                        "type": "error",
                        "error": {
                            "type": "stream_error",
                            "message": format!("Stream error: {error}")
                        }
                    });
                    let sse_data = format!(
                        "event: error\ndata: {}\n\n",
                        serde_json::to_string(&error_event).unwrap_or_default()
                    );
                    yield Ok(Bytes::from(sse_data));
                    return;
                }
            }
        }

        stream_completion.record_success();
    }
}

fn map_stop_reason(finish_reason: Option<&str>) -> Option<String> {
    finish_reason.map(|reason| {
        match reason {
            "tool_calls" => "tool_use",
            "stop" => "end_turn",
            "length" => "max_tokens",
            _ => "end_turn",
        }
        .to_string()
    })
}
