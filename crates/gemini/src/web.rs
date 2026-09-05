//! The browser transport: `fetch` through `web-sys`, on the main thread or in
//! a worker. Behind the `web` feature and only on `wasm32`.
//!
//! This exists for bring-your-own-key clients and single-player builds. A
//! shared world's key stays on the server; browsers talk to the server.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::{sse, Delta, Error, Http, Request, Response, Stream, BASE_URL};

pub struct Client {
    key: String,
    base: String,
}

impl Client {
    pub fn new(key: impl Into<String>) -> Client {
        Client {
            key: key.into(),
            base: BASE_URL.to_string(),
        }
    }
    pub fn base(mut self, base: impl Into<String>) -> Client {
        self.base = base.into();
        self
    }

    pub async fn generate(&self, req: &Request) -> Result<Response, Error> {
        let resp = fetch(&req.http_at(&self.base, &self.key, false)).await?;
        let code = resp.status();
        let text = body_text(&resp).await?;
        if code >= 400 {
            return Err(Error::from_http(code, &text));
        }
        Response::parse(&text)
    }

    pub async fn stream(
        &self,
        req: &Request,
        mut on: impl FnMut(Delta),
    ) -> Result<Response, Error> {
        let resp = fetch(&req.http_at(&self.base, &self.key, true)).await?;
        let code = resp.status();
        if code >= 400 {
            let text = body_text(&resp).await?;
            return Err(Error::from_http(code, &text));
        }
        let body = resp
            .body()
            .ok_or_else(|| Error::Transport("response has no body".into()))?;
        let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().unchecked_into();
        let mut dec = sse::Decoder::new();
        let mut acc = Stream::new();
        loop {
            let chunk = JsFuture::from(reader.read()).await.map_err(js_err)?;
            let done = js_sys::Reflect::get(&chunk, &"done".into())
                .ok()
                .and_then(|d| d.as_bool())
                .unwrap_or(true);
            if done {
                break;
            }
            let value = js_sys::Reflect::get(&chunk, &"value".into()).map_err(js_err)?;
            let bytes = js_sys::Uint8Array::new(&value).to_vec();
            dec.push(&bytes);
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
}

async fn fetch(http: &Http) -> Result<web_sys::Response, Error> {
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&http.body));
    let headers = web_sys::Headers::new().map_err(js_err)?;
    for (k, v) in &http.headers {
        headers.set(k, v).map_err(js_err)?;
    }
    init.set_headers(&headers);
    let request = web_sys::Request::new_with_str_and_init(&http.url, &init).map_err(js_err)?;
    let global = js_sys::global();
    let promise = if let Some(w) = global.dyn_ref::<web_sys::Window>() {
        w.fetch_with_request(&request)
    } else if let Some(w) = global.dyn_ref::<web_sys::WorkerGlobalScope>() {
        w.fetch_with_request(&request)
    } else {
        return Err(Error::Transport("no fetch() in this global scope".into()));
    };
    let resp = JsFuture::from(promise).await.map_err(js_err)?;
    resp.dyn_into::<web_sys::Response>()
        .map_err(|_| Error::Transport("fetch resolved to a non-Response".into()))
}

async fn body_text(resp: &web_sys::Response) -> Result<String, Error> {
    let text = JsFuture::from(resp.text().map_err(js_err)?)
        .await
        .map_err(js_err)?;
    Ok(text.as_string().unwrap_or_default())
}

fn js_err(e: JsValue) -> Error {
    Error::Transport(format!("{e:?}"))
}
