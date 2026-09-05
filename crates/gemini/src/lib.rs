//! Gemini from first principles.
//!
//! The API is small: one `generateContent` call, a streaming twin over SSE,
//! and a handful of JSON shapes. This crate speaks exactly that and nothing
//! else. The core (this file, `json`, `sse`) has **no dependencies** and
//! compiles for native and `wasm32-unknown-unknown` alike; it only builds
//! bytes and parses bytes. The two transports — `native` (ureq) and `web`
//! (fetch) — are feature-gated and about a hundred lines each.
//!
//! ```no_run
//! # #[cfg(feature = "native")] {
//! use gemini::{native::Client, Request};
//! let client = Client::from_env().unwrap();
//! let reply = client.generate(&Request::new("gemini-3.5-flash-lite").user("hi")).unwrap();
//! println!("{}", reply.text());
//! # }
//! ```
//!
//! Wire facts this crate encodes, all probed live against the API (2026-09):
//! - Auth is the `x-goog-api-key` header; the path is
//!   `/v1beta/models/{model}:generateContent`, streaming adds
//!   `:streamGenerateContent?alt=sse`. Streams use CRLF frames and no `[DONE]`.
//! - Gemini 3.x stamps parts with a `thoughtSignature` and rejects replayed
//!   history that lost it, so parts carry the signature through untouched.
//! - `thinkingBudget: 0` is refused by 3.5+ models (`thinkingLevel` is the
//!   knob there) but still works on 2.5 — so `Thinking` is explicit and
//!   nothing is sent unless asked.
//! - A function call now carries an `id`; echo it in the function response.
//! - `includeThoughts` returns a `thought: true` part on `generateContent` but
//!   **not** on the SSE stream (3.5-flash, `thinkingLevel: low`); the thought
//!   token count still arrives in `usageMetadata` either way.
//! - Errors arrive as `{"error": {"code", "status", "message"}}` with the same
//!   HTTP status, on both endpoints.

pub mod json;
#[cfg(feature = "native")]
pub mod native;
pub mod sse;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;

use std::fmt;

pub use json::Value;

pub const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Model,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Model => "model",
        }
    }
}

/// One part of a turn. The model's parts come back with opaque signatures
/// that must be replayed verbatim; `Raw` carries any shape this crate does
/// not model (code execution, file data) through untouched.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text {
        text: String,
        thought: bool,
        signature: Option<String>,
    },
    Call {
        name: String,
        args: Value,
        id: Option<String>,
        signature: Option<String>,
    },
    Result {
        name: String,
        id: Option<String>,
        response: Value,
    },
    Blob {
        mime: String,
        data: String,
    },
    Raw(Value),
}

impl Part {
    pub fn text(text: impl Into<String>) -> Part {
        Part::Text {
            text: text.into(),
            thought: false,
            signature: None,
        }
    }

    /// A function result. The API wants an object; anything else is wrapped
    /// as `{"result": value}`.
    pub fn result(name: impl Into<String>, id: Option<String>, response: Value) -> Part {
        let response = match response {
            v @ Value::Obj(_) => v,
            v => obj! {"result" => v},
        };
        Part::Result {
            name: name.into(),
            id,
            response,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Part::Text {
                text,
                thought,
                signature,
            } => Value::obj()
                .with("text", text.as_str())
                .with_opt("thought", if *thought { Some(true) } else { None })
                .with_opt("thoughtSignature", signature.as_deref()),
            Part::Call {
                name,
                args,
                id,
                signature,
            } => Value::obj()
                .with(
                    "functionCall",
                    Value::obj()
                        .with("name", name.as_str())
                        .with("args", args)
                        .with_opt("id", id.as_deref()),
                )
                .with_opt("thoughtSignature", signature.as_deref()),
            Part::Result { name, id, response } => Value::obj().with(
                "functionResponse",
                Value::obj()
                    .with("name", name.as_str())
                    .with_opt("id", id.as_deref())
                    .with("response", response),
            ),
            Part::Blob { mime, data } => Value::obj().with(
                "inlineData",
                obj! {"mimeType" => mime.as_str(), "data" => data.as_str()},
            ),
            Part::Raw(v) => v.clone(),
        }
    }

    pub fn from_json(v: &Value) -> Part {
        let signature = v.get("thoughtSignature").as_str().map(str::to_string);
        let call = v.get("functionCall");
        if !call.is_null() {
            return Part::Call {
                name: call.get("name").to_text(),
                args: match call.get("args") {
                    Value::Null => Value::obj(),
                    a => a.clone(),
                },
                id: call.get("id").as_str().map(str::to_string),
                signature,
            };
        }
        let res = v.get("functionResponse");
        if !res.is_null() {
            return Part::Result {
                name: res.get("name").to_text(),
                id: res.get("id").as_str().map(str::to_string),
                response: res.get("response").clone(),
            };
        }
        let blob = v.get("inlineData");
        if !blob.is_null() {
            return Part::Blob {
                mime: blob.get("mimeType").to_text(),
                data: blob.get("data").to_text(),
            };
        }
        if let Some(text) = v.get("text").as_str() {
            return Part::Text {
                text: text.to_string(),
                thought: v.get("thought").as_bool() == Some(true),
                signature,
            };
        }
        Part::Raw(v.clone())
    }
}

/// A borrowed view of a function call the model made.
#[derive(Clone, Copy, Debug)]
pub struct Call<'a> {
    pub name: &'a str,
    pub args: &'a Value,
    pub id: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Default for Content {
    fn default() -> Self {
        Content {
            role: Role::Model,
            parts: Vec::new(),
        }
    }
}

impl Content {
    pub fn new(role: Role, parts: Vec<Part>) -> Content {
        Content { role, parts }
    }
    pub fn user(text: impl Into<String>) -> Content {
        Content::new(Role::User, vec![Part::text(text)])
    }
    pub fn model(text: impl Into<String>) -> Content {
        Content::new(Role::Model, vec![Part::text(text)])
    }
    /// The turn that answers the model's function calls.
    pub fn results(results: Vec<Part>) -> Content {
        Content::new(Role::User, results)
    }

    /// Visible text, thoughts excluded, parts concatenated.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for p in &self.parts {
            if let Part::Text {
                text,
                thought: false,
                ..
            } = p
            {
                s.push_str(text);
            }
        }
        s
    }
    pub fn thoughts(&self) -> String {
        let mut s = String::new();
        for p in &self.parts {
            if let Part::Text {
                text,
                thought: true,
                ..
            } = p
            {
                s.push_str(text);
            }
        }
        s
    }
    pub fn calls(&self) -> Vec<Call<'_>> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Call { name, args, id, .. } => Some(Call {
                    name,
                    args,
                    id: id.as_deref(),
                }),
                _ => None,
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        obj! {
            "role" => self.role.as_str(),
            "parts" => self.parts.iter().map(Part::to_json).collect::<Vec<_>>(),
        }
    }
    pub fn from_json(v: &Value) -> Content {
        let role = match v.get("role").as_str() {
            Some("user") => Role::User,
            _ => Role::Model,
        };
        Content {
            role,
            parts: v
                .get("parts")
                .as_arr()
                .iter()
                .map(Part::from_json)
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// A function the model may call. `parameters` is a JSON Schema object.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub description: String,
    pub parameters: Option<Value>,
}

impl Function {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Function {
        Function {
            name: name.into(),
            description: description.into(),
            parameters: None,
        }
    }
    pub fn params(mut self, schema: Value) -> Function {
        self.parameters = Some(schema);
        self
    }
    fn to_json(&self) -> Value {
        obj! {"name" => self.name.as_str(), "description" => self.description.as_str()}
            .with_opt("parameters", self.parameters.as_ref())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolMode {
    /// The model decides whether to call anything.
    Auto,
    /// The model must call a function.
    Any,
    /// The model must not.
    None,
}

/// Reasoning effort. Nothing is sent for `Default`. `Budget` is the 2.5-era
/// token budget (0 = off) and is refused by 3.5+ models; `Level` is the 3.x
/// knob (`Minimal` is refused by some 3.x models — start at `Low`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thinking {
    Default,
    Budget(u32),
    Level(Level),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Minimal,
    Low,
    Medium,
    High,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Minimal => "minimal",
            Level::Low => "low",
            Level::Medium => "medium",
            Level::High => "high",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_output_tokens: Option<u32>,
    pub stop: Vec<String>,
    /// Structured output: sets `responseMimeType: application/json` and the schema.
    pub json_schema: Option<Value>,
    pub thinking: Thinking,
    pub include_thoughts: bool,
    pub seed: Option<i64>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            stop: Vec::new(),
            json_schema: None,
            thinking: Thinking::Default,
            include_thoughts: false,
            seed: None,
        }
    }
}

/// Everything one `generateContent` call needs, minus the key.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub model: String,
    pub system: Option<String>,
    pub contents: Vec<Content>,
    pub tools: Vec<Function>,
    pub tool_mode: Option<ToolMode>,
    pub config: Config,
}

/// A transport-agnostic HTTP request: what any client must send.
#[derive(Clone, Debug, PartialEq)]
pub struct Http {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl Request {
    pub fn new(model: impl Into<String>) -> Request {
        Request {
            model: model.into(),
            system: None,
            contents: Vec::new(),
            tools: Vec::new(),
            tool_mode: None,
            config: Config::default(),
        }
    }
    pub fn system(mut self, text: impl Into<String>) -> Request {
        self.system = Some(text.into());
        self
    }
    /// Append a user turn.
    pub fn user(mut self, text: impl Into<String>) -> Request {
        self.contents.push(Content::user(text));
        self
    }
    pub fn content(mut self, c: Content) -> Request {
        self.contents.push(c);
        self
    }
    pub fn tool(mut self, f: Function) -> Request {
        self.tools.push(f);
        self
    }
    pub fn tools(mut self, fs: impl IntoIterator<Item = Function>) -> Request {
        self.tools.extend(fs);
        self
    }
    pub fn tool_mode(mut self, m: ToolMode) -> Request {
        self.tool_mode = Some(m);
        self
    }
    pub fn temperature(mut self, t: f32) -> Request {
        self.config.temperature = Some(t);
        self
    }
    pub fn max_tokens(mut self, n: u32) -> Request {
        self.config.max_output_tokens = Some(n);
        self
    }
    pub fn json(mut self, schema: Value) -> Request {
        self.config.json_schema = Some(schema);
        self
    }
    pub fn thinking(mut self, t: Thinking) -> Request {
        self.config.thinking = t;
        self
    }
    pub fn include_thoughts(mut self, on: bool) -> Request {
        self.config.include_thoughts = on;
        self
    }

    /// The JSON body, exactly as the API wants it.
    pub fn body(&self) -> Value {
        let mut body = Value::obj();
        if let Some(s) = &self.system {
            body.set(
                "systemInstruction",
                obj! {"parts" => arr![obj! {"text" => s.as_str()}]},
            );
        }
        body.set(
            "contents",
            self.contents
                .iter()
                .map(Content::to_json)
                .collect::<Vec<_>>(),
        );
        if !self.tools.is_empty() {
            let decls: Vec<Value> = self.tools.iter().map(Function::to_json).collect();
            body.set("tools", arr![obj! {"functionDeclarations" => decls}]);
        }
        if let Some(mode) = self.tool_mode {
            let mode = match mode {
                ToolMode::Auto => "AUTO",
                ToolMode::Any => "ANY",
                ToolMode::None => "NONE",
            };
            body.set(
                "toolConfig",
                obj! {"functionCallingConfig" => obj! {"mode" => mode}},
            );
        }
        let c = &self.config;
        let mut gen = Value::obj()
            .with_opt("temperature", c.temperature)
            .with_opt("topP", c.top_p)
            .with_opt("maxOutputTokens", c.max_output_tokens)
            .with_opt("seed", c.seed);
        if !c.stop.is_empty() {
            gen.set("stopSequences", c.stop.clone());
        }
        if let Some(schema) = &c.json_schema {
            gen.set("responseMimeType", "application/json");
            gen.set("responseSchema", schema);
        }
        let mut thinking = Value::obj();
        match c.thinking {
            Thinking::Default => {}
            Thinking::Budget(n) => thinking.set("thinkingBudget", n),
            Thinking::Level(l) => thinking.set("thinkingLevel", l.as_str()),
        }
        if c.include_thoughts {
            thinking.set("includeThoughts", true);
        }
        if !thinking.as_obj().is_empty() {
            gen.set("thinkingConfig", thinking);
        }
        if !gen.as_obj().is_empty() {
            body.set("generationConfig", gen);
        }
        body
    }

    pub fn http(&self, key: &str, stream: bool) -> Http {
        self.http_at(BASE_URL, key, stream)
    }

    pub fn http_at(&self, base: &str, key: &str, stream: bool) -> Http {
        let url = if stream {
            format!("{base}/models/{}:streamGenerateContent?alt=sse", self.model)
        } else {
            format!("{base}/models/{}:generateContent", self.model)
        };
        Http {
            url,
            headers: vec![
                ("x-goog-api-key", key.to_string()),
                ("content-type", "application/json".to_string()),
            ],
            body: self.body().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finish {
    Stop,
    MaxTokens,
    Safety,
    Recitation,
    Blocklist,
    ProhibitedContent,
    Spii,
    MalformedCall,
    Other(String),
}

impl Finish {
    fn parse(s: &str) -> Finish {
        match s {
            "STOP" => Finish::Stop,
            "MAX_TOKENS" => Finish::MaxTokens,
            "SAFETY" => Finish::Safety,
            "RECITATION" => Finish::Recitation,
            "BLOCKLIST" => Finish::Blocklist,
            "PROHIBITED_CONTENT" => Finish::ProhibitedContent,
            "SPII" => Finish::Spii,
            "MALFORMED_FUNCTION_CALL" => Finish::MalformedCall,
            other => Finish::Other(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt: u32,
    pub output: u32,
    pub thoughts: u32,
    pub cached: u32,
    pub total: u32,
}

/// One model turn, whether it arrived whole or was accumulated from a stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub content: Content,
    pub finish: Option<Finish>,
    pub finish_message: Option<String>,
    /// The *prompt* was refused (`promptFeedback.blockReason`); there is no candidate.
    pub blocked: Option<String>,
    pub usage: Usage,
    pub model_version: Option<String>,
    pub id: Option<String>,
}

impl Response {
    /// Parse a full (non-streaming) response body. An API error body becomes `Error::Api`.
    pub fn parse(json: &str) -> Result<Response, Error> {
        let v = Value::parse(json)?;
        Response::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<Response, Error> {
        if let Some(e) = Error::from_body(v) {
            return Err(e);
        }
        let cand = v.get("candidates").at(0);
        let u = v.get("usageMetadata");
        Ok(Response {
            content: Content::from_json(cand.get("content")),
            finish: cand.get("finishReason").as_str().map(Finish::parse),
            finish_message: cand.get("finishMessage").as_str().map(str::to_string),
            blocked: v
                .get("promptFeedback")
                .get("blockReason")
                .as_str()
                .map(str::to_string),
            usage: Usage {
                prompt: u.get("promptTokenCount").as_u32().unwrap_or(0),
                output: u.get("candidatesTokenCount").as_u32().unwrap_or(0),
                thoughts: u.get("thoughtsTokenCount").as_u32().unwrap_or(0),
                cached: u.get("cachedContentTokenCount").as_u32().unwrap_or(0),
                total: u.get("totalTokenCount").as_u32().unwrap_or(0),
            },
            model_version: v.get("modelVersion").as_str().map(str::to_string),
            id: v.get("responseId").as_str().map(str::to_string),
        })
    }

    pub fn text(&self) -> String {
        self.content.text()
    }
    pub fn thoughts(&self) -> String {
        self.content.thoughts()
    }
    pub fn calls(&self) -> Vec<Call<'_>> {
        self.content.calls()
    }
    /// The text parsed as JSON — for structured output requests.
    pub fn json(&self) -> Result<Value, Error> {
        Ok(Value::parse(&self.text())?)
    }
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// What one streamed chunk added.
#[derive(Clone, Debug, PartialEq)]
pub enum Delta {
    Text(String),
    Thought(String),
    Call {
        name: String,
        args: Value,
        id: Option<String>,
    },
}

/// Folds streamed chunks into one `Response`, so a stream ends with exactly
/// the turn a non-streaming call would have returned (signatures included).
#[derive(Default)]
pub struct Stream {
    response: Response,
}

impl Stream {
    pub fn new() -> Stream {
        Stream::default()
    }

    /// Feed one SSE `data:` payload; get back what it carried.
    pub fn chunk(&mut self, payload: &str) -> Result<Vec<Delta>, Error> {
        let v = Value::parse(payload)?;
        let part = Response::from_value(&v)?;
        let mut deltas = Vec::new();
        for p in part.content.parts {
            match p {
                Part::Text {
                    text,
                    thought,
                    signature,
                } => {
                    if !text.is_empty() {
                        deltas.push(if thought {
                            Delta::Thought(text.clone())
                        } else {
                            Delta::Text(text.clone())
                        });
                    }
                    match self.response.content.parts.last_mut() {
                        Some(Part::Text {
                            text: acc,
                            thought: t,
                            signature: sig,
                        }) if *t == thought => {
                            acc.push_str(&text);
                            if signature.is_some() {
                                *sig = signature;
                            }
                        }
                        _ => {
                            if !text.is_empty() || signature.is_some() {
                                self.response.content.parts.push(Part::Text {
                                    text,
                                    thought,
                                    signature,
                                });
                            }
                        }
                    }
                }
                Part::Call {
                    name,
                    args,
                    id,
                    signature,
                } => {
                    deltas.push(Delta::Call {
                        name: name.clone(),
                        args: args.clone(),
                        id: id.clone(),
                    });
                    self.response.content.parts.push(Part::Call {
                        name,
                        args,
                        id,
                        signature,
                    });
                }
                other => self.response.content.parts.push(other),
            }
        }
        let r = &mut self.response;
        r.content.role = part.content.role;
        if part.finish.is_some() {
            r.finish = part.finish;
        }
        if part.finish_message.is_some() {
            r.finish_message = part.finish_message;
        }
        if part.blocked.is_some() {
            r.blocked = part.blocked;
        }
        if part.usage != Usage::default() {
            r.usage = part.usage;
        }
        if part.model_version.is_some() {
            r.model_version = part.model_version;
        }
        if part.id.is_some() {
            r.id = part.id;
        }
        Ok(deltas)
    }

    pub fn finish(self) -> Response {
        self.response
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The API said no: HTTP status, gRPC-style status name, message.
    Api {
        code: u16,
        status: String,
        message: String,
    },
    Json(json::JsonError),
    Transport(String),
}

impl Error {
    /// `{"error": {...}}` as the API wires it, if that is what `v` is.
    pub fn from_body(v: &Value) -> Option<Error> {
        let e = v.get("error");
        if e.is_null() {
            return None;
        }
        Some(Error::Api {
            code: e.get("code").as_u32().unwrap_or(0) as u16,
            status: e.get("status").to_text(),
            message: e.get("message").to_text(),
        })
    }
    /// A non-2xx body that may or may not be the JSON error shape.
    pub fn from_http(code: u16, body: &str) -> Error {
        match Value::parse(body).ok().and_then(|v| Error::from_body(&v)) {
            Some(e) => e,
            None => Error::Api {
                code,
                status: String::new(),
                message: body.chars().take(400).collect(),
            },
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api {
                code,
                status,
                message,
            } => write!(f, "gemini {code} {status}: {message}"),
            Error::Json(e) => write!(f, "gemini: {e}"),
            Error::Transport(e) => write!(f, "gemini transport: {e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<json::JsonError> for Error {
    fn from(e: json::JsonError) -> Error {
        Error::Json(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_is_exactly_the_wire_shape() {
        let req = Request::new("m")
            .system("sys")
            .user("hi")
            .tool(Function::new("f", "d").params(obj! {"type" => "object"}))
            .tool_mode(ToolMode::Any)
            .temperature(0.5)
            .thinking(Thinking::Level(Level::Low));
        assert_eq!(
            req.body().to_string(),
            r#"{"systemInstruction":{"parts":[{"text":"sys"}]},"contents":[{"role":"user","parts":[{"text":"hi"}]}],"tools":[{"functionDeclarations":[{"name":"f","description":"d","parameters":{"type":"object"}}]}],"toolConfig":{"functionCallingConfig":{"mode":"ANY"}},"generationConfig":{"temperature":0.5,"thinkingConfig":{"thinkingLevel":"low"}}}"#
        );
        let h = req.http("KEY", true);
        assert!(h.url.ends_with("/models/m:streamGenerateContent?alt=sse"));
        assert_eq!(h.headers[0], ("x-goog-api-key", "KEY".to_string()));
        // A bare request sends no generationConfig at all.
        assert_eq!(
            Request::new("m").user("x").body().get("generationConfig"),
            &Value::Null
        );
    }

    #[test]
    fn parses_a_function_call_response() {
        let body = r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"move_to","args":{"target":"north"},"id":"call_1"},"thoughtSignature":"SIG"}],"role":"model"},"finishReason":"STOP","index":0,"finishMessage":"Model generated function call(s)."}],"usageMetadata":{"promptTokenCount":210,"candidatesTokenCount":16,"totalTokenCount":226},"modelVersion":"gemini-3.5-flash-lite","responseId":"abc"}"#;
        let r = Response::parse(body).unwrap();
        let calls = r.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "move_to");
        assert_eq!(calls[0].args.get("target").as_str(), Some("north"));
        assert_eq!(calls[0].id, Some("call_1"));
        assert_eq!(r.finish, Some(Finish::Stop));
        assert_eq!(r.usage.total, 226);
        // The model turn replays with its signature intact.
        let replay = r.content.to_json().to_string();
        assert!(replay.contains("\"thoughtSignature\":\"SIG\""));
        assert!(replay.contains("\"id\":\"call_1\""));
    }

    #[test]
    fn thoughts_and_blocked_prompts() {
        let body = r#"{"candidates":[{"content":{"parts":[{"text":"hmm","thought":true},{"text":"391","thoughtSignature":"S"}],"role":"model"},"finishReason":"STOP"}]}"#;
        let r = Response::parse(body).unwrap();
        assert_eq!(r.text(), "391");
        assert_eq!(r.thoughts(), "hmm");
        let blocked =
            r#"{"promptFeedback":{"blockReason":"SAFETY"},"usageMetadata":{"promptTokenCount":3}}"#;
        let r = Response::parse(blocked).unwrap();
        assert_eq!(r.blocked.as_deref(), Some("SAFETY"));
        assert!(r.content.parts.is_empty());
        // A content-filtered candidate wires `"content": {}` — must not fail.
        let filtered = r#"{"candidates":[{"content":{},"finishReason":"SAFETY"}]}"#;
        assert_eq!(
            Response::parse(filtered).unwrap().finish,
            Some(Finish::Safety)
        );
    }

    #[test]
    fn api_errors_surface_as_errors() {
        let e = Response::parse(r#"{"error":{"code":404,"message":"nope","status":"NOT_FOUND"}}"#)
            .unwrap_err();
        assert_eq!(
            e,
            Error::Api {
                code: 404,
                status: "NOT_FOUND".into(),
                message: "nope".into()
            }
        );
        assert!(matches!(
            Error::from_http(502, "<html>bad gateway</html>"),
            Error::Api { code: 502, .. }
        ));
    }

    #[test]
    fn stream_folds_into_one_turn() {
        let chunks = [
            r#"{"candidates":[{"content":{"parts":[{"text":"1 "}],"role":"model"},"index":0}],"usageMetadata":{"totalTokenCount":15}}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"2 3"}],"role":"model"},"index":0}],"usageMetadata":{"totalTokenCount":33}}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"","thoughtSignature":"SIG"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"totalTokenCount":39}}"#,
        ];
        let mut s = Stream::new();
        let mut seen = Vec::new();
        for c in chunks {
            seen.extend(s.chunk(c).unwrap());
        }
        assert_eq!(
            seen,
            vec![Delta::Text("1 ".into()), Delta::Text("2 3".into())]
        );
        let r = s.finish();
        assert_eq!(r.text(), "1 2 3");
        assert_eq!(r.content.parts.len(), 1);
        assert_eq!(
            r.content.parts[0],
            Part::Text {
                text: "1 2 3".into(),
                thought: false,
                signature: Some("SIG".into())
            }
        );
        assert_eq!(r.finish, Some(Finish::Stop));
        assert_eq!(r.usage.total, 39);
    }

    #[test]
    fn unknown_parts_pass_through() {
        let v = Value::parse(r#"{"executableCode":{"language":"PYTHON","code":"x"}}"#).unwrap();
        let p = Part::from_json(&v);
        assert_eq!(p, Part::Raw(v.clone()));
        assert_eq!(p.to_json(), v);
        let r = Part::result("f", Some("id1".into()), Value::from(42));
        assert_eq!(
            r.to_json().to_string(),
            r#"{"functionResponse":{"name":"f","id":"id1","response":{"result":42}}}"#
        );
    }
}
