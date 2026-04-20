use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

// ── Tool definition types ───────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDef,
}

#[derive(Clone, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── Message types ───────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(), content: Some(serde_json::Value::String(content.into())),
            tool_calls: None, tool_call_id: None, extra: Default::default(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(), content: Some(serde_json::Value::String(content.into())),
            tool_calls: None, tool_call_id: None, extra: Default::default(),
        }
    }

    /// User message with OpenAI multimodal content (array of {type,text}/{type,image_url} parts).
    pub fn user_parts(parts: serde_json::Value) -> Self {
        Self {
            role: "user".into(), content: Some(parts),
            tool_calls: None, tool_call_id: None, extra: Default::default(),
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(), content: Some(serde_json::Value::String(content.into())),
            tool_calls: None, tool_call_id: Some(id.into()), extra: Default::default(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

// ── Usage ──────────────────────────────────────────────────────────────

#[derive(Clone, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

// ── Traits ──────────────────────────────────────────────────────────────

pub trait ToolExecutor: Send + Sync {
    fn definitions(&self) -> Vec<ToolDef>;

    fn execute(
        &self, name: &str, args: &serde_json::Value,
    ) -> impl std::future::Future<Output = String> + Send;

    fn continuation_check(&self) -> Option<String> {
        None
    }

    fn should_stop(&self) -> bool {
        false
    }
}

/// Handler for events produced during the ReAct loop.
pub trait ReactHandler {
    fn on_llm_request(&self, _turn: usize, _message_count: usize) {}
    fn on_llm_response(&self, _turn: usize, _content: &str, _tool_call_count: usize) {}
    fn on_llm_error(&self, _turn: usize, _error: &str) {}
    fn on_text(&self, _text: &str) {}
    fn on_tool_call(&self, _name: &str, _args: &serde_json::Value) {}
    fn on_tool_result(&self, _name: &str, _result: &str) {}
    fn on_turn_complete(&self, _turn: usize, _message_count: usize, _usage: &Usage) {}
}

// ── Streaming SSE types ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct StreamChoice { delta: StreamDelta }

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Deserialize)]
struct StreamToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunction>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct StreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

struct StreamResult {
    content: String,
    tool_calls: Vec<ToolCall>,
    usage: Usage,
}

async fn read_stream(
    resp: reqwest::Response,
    handler: &impl ReactHandler,
) -> Result<StreamResult, Box<dyn std::error::Error>> {
    let mut content = String::new();
    let mut tc_ids: Vec<String> = vec![];
    let mut tc_names: Vec<String> = vec![];
    let mut tc_args: Vec<String> = vec![];
    let mut tc_extras: Vec<serde_json::Map<String, serde_json::Value>> = vec![];
    let mut usage = Usage::default();

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 1..].to_string();

            if !line.starts_with("data: ") { continue; }
            let data = &line[6..];
            if data == "[DONE]" { break; }

            let chunk: StreamChunk = match serde_json::from_str(data) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(u) = chunk.usage {
                usage = u;
            }

            for choice in &chunk.choices {
                if let Some(ref text) = choice.delta.content {
                    handler.on_text(text);
                    content.push_str(text);
                }

                if let Some(ref calls) = choice.delta.tool_calls {
                    for tc in calls {
                        let idx = tc.index;
                        while tc_ids.len() <= idx {
                            tc_ids.push(String::new());
                            tc_names.push(String::new());
                            tc_args.push(String::new());
                            tc_extras.push(serde_json::Map::new());
                        }
                        if let Some(ref id) = tc.id { tc_ids[idx] = id.clone(); }
                        if let Some(ref func) = tc.function {
                            if let Some(ref name) = func.name { tc_names[idx] = name.clone(); }
                            if let Some(ref args) = func.arguments { tc_args[idx].push_str(args); }
                        }
                        for (k, v) in &tc.extra {
                            tc_extras[idx].insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }

    let tool_calls: Vec<ToolCall> = (0..tc_ids.len())
        .filter(|i| !tc_names[*i].is_empty())
        .map(|i| ToolCall {
            id: tc_ids[i].clone(),
            kind: "function".into(),
            function: FunctionCall { name: tc_names[i].clone(), arguments: tc_args[i].clone() },
            extra: {
                let mut e = tc_extras[i].clone();
                e.remove("type");
                e
            },
        })
        .collect();

    Ok(StreamResult { content, tool_calls, usage })
}

// ── ReAct loop ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct ChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    messages: Vec<Message>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

pub struct ReactConfig {
    pub llm_url: String,
    pub api_key: String,
    pub model: String,
    pub max_turns: usize,
    pub reasoning_effort: Option<String>,
}

pub struct ReactResult {
    pub response: String,
    pub messages: Vec<Message>,
    pub total_tokens: u64,
}

pub async fn run(
    config: &ReactConfig,
    tools: &impl ToolExecutor,
    mut messages: Vec<Message>,
    handler: &impl ReactHandler,
) -> Result<ReactResult, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let tool_defs = tools.definitions();
    let has_tools = !tool_defs.is_empty();
    let mut total_tokens: u64 = 0;

    for turn in 0..config.max_turns {
        handler.on_llm_request(turn, messages.len());

        let mut req = client
            .post(format!("{}/chat/completions", config.llm_url));
        // Skip Authorization entirely when no key is configured — typically
        // means a proxy in front of the real provider handles auth.
        if !config.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", config.api_key));
        }
        let resp = req
            .json(&ChatRequest {
                model: Some(config.model.clone()),
                messages: messages.clone(),
                stream: true,
                stream_options: StreamOptions { include_usage: true },
                tools: if has_tools { Some(tool_defs.clone()) } else { None },
                tool_choice: None,
                reasoning_effort: config.reasoning_effort.clone(),
            })
            .send().await?;

        if !resp.status().is_success() {
            let s = resp.status();
            let body = resp.text().await.unwrap_or_default();
            handler.on_llm_error(turn, &format!("{s}: {body}"));
            return Err(format!("API error ({s}): {body}").into());
        }

        let mut result = read_stream(resp, handler).await?;
        total_tokens += result.usage.total_tokens;

        handler.on_llm_response(turn, &result.content, result.tool_calls.len());

        // Normalize tool call arguments before adding to history
        for tc in &mut result.tool_calls {
            let parsed: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
            tc.function.arguments = serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".into());
        }

        let reply = Message {
            role: "assistant".into(),
            content: if result.content.is_empty() { None } else { Some(serde_json::Value::String(result.content.clone())) },
            tool_calls: if result.tool_calls.is_empty() { None } else { Some(result.tool_calls.clone()) },
            tool_call_id: None,
            extra: Default::default(),
        };
        messages.push(reply);

        // No tool calls — model is done
        if result.tool_calls.is_empty() {
            if let Some(reminder) = tools.continuation_check() {
                messages.push(Message::user(reminder));
                continue;
            }

            handler.on_turn_complete(turn, messages.len(), &result.usage);
            return Ok(ReactResult { response: result.content, messages, total_tokens });
        }

        // Execute tool calls
        for tc in &result.tool_calls {
            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));

            handler.on_tool_call(&tc.function.name, &args);
            let tool_output = tools.execute(&tc.function.name, &args).await;
            handler.on_tool_result(&tc.function.name, &tool_output);

            let tool_msg = Message::tool_result(&tc.id, tool_output);
            messages.push(tool_msg);
        }

        handler.on_turn_complete(turn, messages.len(), &result.usage);

        if tools.should_stop() {
            return Ok(ReactResult { response: result.content, messages, total_tokens });
        }
    }

    Err("Max turns exceeded".into())
}
