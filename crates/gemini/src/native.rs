//! The native transport: blocking HTTPS over `ureq`. This module is the only
//! reason the crate has a dependency, and it is behind the `native` feature.
//!
//! Blocking is a choice, not a shortcut: a game server calling a model from a
//! worker thread wants a plain function that returns, and a stream callback
//! that fires as bytes land. Anything async can wrap this in `spawn_blocking`.

use std::io::Read;
use std::time::Duration;

use crate::{sse, Delta, Error, Http, Request, Response, Stream, BASE_URL};

/// Cheap to clone: the agent shares its connection pool.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    key: String,
    base: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key never reaches a log.
        f.debug_struct("Client").field("base", &self.base).finish()
    }
}

impl Client {
    pub fn new(key: impl Into<String>) -> Client {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(300)))
            .build();
        Client {
            agent: config.into(),
            key: key.into(),
            base: BASE_URL.to_string(),
        }
    }

    /// Reads `GEMINI_API_KEY`, loading a `.env` from the working directory or
    /// any parent first.
    pub fn from_env() -> Result<Client, Error> {
        dotenv();
        match std::env::var("GEMINI_API_KEY") {
            Ok(k) if !k.trim().is_empty() => Ok(Client::new(k.trim().to_string())),
            _ => Err(Error::Transport("GEMINI_API_KEY is not set".into())),
        }
    }

    /// Point at a proxy or a different API version.
    pub fn base(mut self, base: impl Into<String>) -> Client {
        self.base = base.into();
        self
    }

    /// One call, one turn.
    pub fn generate(&self, req: &Request) -> Result<Response, Error> {
        let mut resp = self.send(&req.http_at(&self.base, &self.key, false))?;
        let code = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(transport)?;
        if code >= 400 {
            return Err(Error::from_http(code, &text));
        }
        Response::parse(&text)
    }

    /// Stream a turn: `on` sees each delta as it arrives; the return value is
    /// the whole turn, identical to what `generate` would have produced.
    pub fn stream(&self, req: &Request, mut on: impl FnMut(Delta)) -> Result<Response, Error> {
        let mut resp = self.send(&req.http_at(&self.base, &self.key, true))?;
        let code = resp.status().as_u16();
        if code >= 400 {
            let text = resp.body_mut().read_to_string().map_err(transport)?;
            return Err(Error::from_http(code, &text));
        }
        let mut reader = resp.body_mut().as_reader();
        let mut dec = sse::Decoder::new();
        let mut acc = Stream::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf).map_err(transport)?;
            if n == 0 {
                break;
            }
            dec.push(&buf[..n]);
            while let Some(payload) = dec.next() {
                for d in acc.chunk(&payload)? {
                    on(d);
                }
            }
        }
        if let Some(payload) = dec.finish() {
            for d in acc.chunk(&payload)? {
                on(d);
            }
        }
        Ok(acc.finish())
    }

    /// Model names this key can use, e.g. `gemini-3.5-flash-lite`.
    pub fn models(&self) -> Result<Vec<String>, Error> {
        let url = format!("{}/models?pageSize=200", self.base);
        let mut resp = self
            .agent
            .get(&url)
            .header("x-goog-api-key", &self.key)
            .call()
            .map_err(transport)?;
        let code = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(transport)?;
        if code >= 400 {
            return Err(Error::from_http(code, &text));
        }
        let v = crate::Value::parse(&text)?;
        Ok(v.get("models")
            .as_arr()
            .iter()
            .filter_map(|m| m.get("name").as_str())
            .map(|n| n.trim_start_matches("models/").to_string())
            .collect())
    }

    fn send(&self, http: &Http) -> Result<ureq::http::Response<ureq::Body>, Error> {
        let mut r = self.agent.post(&http.url);
        for (k, v) in &http.headers {
            r = r.header(*k, v.as_str());
        }
        r.send(http.body.as_str()).map_err(transport)
    }
}

fn transport(e: impl std::fmt::Display) -> Error {
    Error::Transport(e.to_string())
}

/// Load `KEY=value` lines from the nearest `.env` (working directory, then
/// each parent) into the process environment. Existing variables win. Quiet
/// when there is no file: a deployed binary gets its environment elsewhere.
pub fn dotenv() {
    let mut dir = std::env::current_dir().ok();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(d.join(".env")) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let k = k.trim().trim_start_matches("export ").trim();
                let v = v.trim().trim_matches('"').trim_matches('\'');
                if std::env::var_os(k).is_none() {
                    std::env::set_var(k, v);
                }
            }
            return;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
}
