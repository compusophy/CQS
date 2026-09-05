//! The browser client. Rust all the way down to the DOM: the page has a name
//! box, a prompt box, the map, the view and a log, and this crate wires them.
//!
//! Identity, for now, is a random token in `localStorage` — enough to test a
//! shared world with; a real login replaces it later without touching the
//! server's idea of a player (a token is a token).

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Document, HtmlInputElement, Window};

use gemini::{obj, Value};

const API: &str = "/api/world";
const POLL_MS: i32 = 3000;

#[wasm_bindgen(start)]
pub fn start() {
    spawn_local(async {
        if let Err(e) = main().await {
            web_sys::console::error_1(&e);
        }
    });
}

fn window() -> Window {
    web_sys::window().expect("a window")
}

fn document() -> Document {
    window().document().expect("a document")
}

fn set_text(id: &str, text: &str) {
    if let Some(el) = document().get_element_by_id(id) {
        el.set_text_content(Some(text));
    }
}

fn input(id: &str) -> Option<HtmlInputElement> {
    document()
        .get_element_by_id(id)?
        .dyn_into::<HtmlInputElement>()
        .ok()
}

fn show(id: &str, on: bool) {
    if let Some(el) = document().get_element_by_id(id) {
        let _ = el
            .dyn_into::<web_sys::HtmlElement>()
            .map(|e| e.set_hidden(!on));
    }
}

fn storage() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

fn stored(key: &str) -> Option<String> {
    storage()?
        .get_item(key)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

fn store(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

/// A fresh secret: 16 random bytes, hex.
fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(crypto) = window().crypto() {
        let _ = crypto.get_random_values_with_u8_array(&mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

async fn fetch_json(method: &str, url: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    if let Some(b) = body {
        init.set_body(&JsValue::from_str(&b.to_string()));
        let headers = web_sys::Headers::new().map_err(js_err)?;
        headers
            .set("content-type", "application/json")
            .map_err(js_err)?;
        init.set_headers(&headers);
    }
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js_err)?;
    let resp = JsFuture::from(window().fetch_with_request(&request))
        .await
        .map_err(js_err)?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "not a response".to_string())?;
    let status = resp.status();
    let text = JsFuture::from(resp.text().map_err(js_err)?)
        .await
        .map_err(js_err)?;
    let text = text.as_string().unwrap_or_default();
    let v = Value::parse(&text)
        .unwrap_or_else(|_| obj! {"error" => text.chars().take(200).collect::<String>()});
    Ok((status, v))
}

fn js_err(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

fn render(v: &Value) {
    if let Some(map) = v.get("map").as_str() {
        set_text("map", map);
    }
    if let Some(view) = v.get("view").as_str() {
        set_text("view", view);
    }
    if let Some(name) = v.get("name").as_str() {
        set_text(
            "who",
            &format!(
                "{name} · tick {}",
                v.get("tick").as_f64().unwrap_or(0.0) as u64
            ),
        );
    } else {
        set_text(
            "who",
            &format!(
                "watching · tick {}",
                v.get("tick").as_f64().unwrap_or(0.0) as u64
            ),
        );
    }
    for line in v.get("said").as_arr() {
        if let Some(s) = line.as_str() {
            log(s);
        }
    }
}

fn log(line: &str) {
    let doc = document();
    let Some(el) = doc.get_element_by_id("log") else {
        return;
    };
    if let Ok(p) = doc.create_element("div") {
        p.set_text_content(Some(line));
        let _ = el.insert_before(&p, el.first_child().as_ref());
        // Keep the log short.
        while el.child_element_count() > 40 {
            if let Some(last) = el.last_element_child() {
                last.remove();
            }
        }
    }
}

async fn main() -> Result<(), JsValue> {
    let token = match stored("cqs.token") {
        Some(t) => t,
        None => {
            let t = new_token();
            store("cqs.token", &t);
            t
        }
    };
    let name = stored("cqs.name");
    show("join", name.is_none());
    show("play", name.is_some());

    // Joining: a name, once.
    {
        let token = token.clone();
        let on_join =
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
                if e.key() != "Enter" {
                    return;
                }
                let Some(field) = input("name") else { return };
                let name = field.value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let token = token.clone();
                spawn_local(async move {
                    set_text("status", "joining…");
                    match fetch_json(
                        "POST",
                        API,
                        Some(&obj! {"token" => token.as_str(), "name" => name.as_str()}),
                    )
                    .await
                    {
                        Ok((200, v)) => {
                            store("cqs.name", v.get("name").as_str().unwrap_or(&name));
                            show("join", false);
                            show("play", true);
                            set_text("status", "");
                            if let Some(ack) = v.get("ack").as_str() {
                                log(ack);
                            }
                            render(&v);
                            if let Some(f) = input("say") {
                                let _ = f.focus();
                            }
                        }
                        Ok((_, v)) => set_text(
                            "status",
                            v.get("error").as_str().unwrap_or("could not join"),
                        ),
                        Err(e) => set_text("status", &e),
                    }
                });
            });
        if let Some(field) = input("name") {
            field.add_event_listener_with_callback("keydown", on_join.as_ref().unchecked_ref())?;
            let _ = field.focus();
        }
        on_join.forget();
    }

    // Speaking: a line, whenever.
    {
        let token = token.clone();
        let on_say =
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
                if e.key() != "Enter" {
                    return;
                }
                let Some(field) = input("say") else { return };
                let words = field.value().trim().to_string();
                if words.is_empty() {
                    return;
                }
                field.set_value("");
                log(&format!("> {words}"));
                let token = token.clone();
                spawn_local(async move {
                    set_text("status", "piloting…");
                    match fetch_json(
                        "POST",
                        API,
                        Some(&obj! {"token" => token.as_str(), "words" => words.as_str()}),
                    )
                    .await
                    {
                        Ok((200, v)) => {
                            set_text("status", "");
                            let pilot = v.get("pilot").as_str().unwrap_or("");
                            if !pilot.is_empty() {
                                log(&format!(
                                    "  pilot: {pilot}  [{} ms]",
                                    v.get("ms").as_f64().unwrap_or(0.0) as u64
                                ));
                            }
                            if let Some(ack) = v.get("ack").as_str() {
                                for line in ack.lines() {
                                    log(line);
                                }
                            }
                            render(&v);
                        }
                        Ok((401, _)) => {
                            // The server forgot us (a reset): join again.
                            store("cqs.name", "");
                            show("join", true);
                            show("play", false);
                            set_text("status", "the world was reset — pick a name again");
                        }
                        Ok((_, v)) => set_text(
                            "status",
                            v.get("error").as_str().unwrap_or("something went wrong"),
                        ),
                        Err(e) => set_text("status", &e),
                    }
                });
            });
        if let Some(field) = input("say") {
            field.add_event_listener_with_callback("keydown", on_say.as_ref().unchecked_ref())?;
        }
        on_say.forget();
    }

    // Watching: the world moves whether or not we type.
    loop {
        let url = format!("{API}?token={token}");
        match fetch_json("GET", &url, None).await {
            Ok((200, v)) => render(&v),
            Ok((_, v)) => set_text("status", v.get("error").as_str().unwrap_or("…")),
            Err(e) => set_text("status", &e),
        }
        sleep(POLL_MS).await;
    }
}
