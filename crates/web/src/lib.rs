//! The browser client. Rust all the way down to the pixels: the page has a
//! header for your character, a square canvas, a prompt box and a console,
//! and this crate wires them. The world is rasterized into a framebuffer by
//! `draw` and presented whole with `putImageData`; the browser draws nothing.
//!
//! Identity, for now, is a random token in `localStorage` — enough to test a
//! shared world with; a real login replaces it later without touching the
//! server's idea of a player (a token is a token).
//!
//! `?demo` runs a local world in the tab with no server at all, which is how
//! the renderer is looked at during development.

mod arch;
mod draw;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, HtmlInputElement, Window};

use draw::{Frame, Scene, VIEW};
use gemini::{obj, Value};

const API: &str = "/api/world";
const POLL_MS: i32 = 3000;
const CONSOLE_LINES: u32 = 200;

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

fn now() -> f64 {
    window().performance().map(|p| p.now()).unwrap_or(0.0)
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

// ---------------------------------------------------------------------------
// The display: a framebuffer, two scenes to interpolate between, a clock
// ---------------------------------------------------------------------------

struct Display {
    ctx: CanvasRenderingContext2d,
    frame: Frame,
    prev: Option<Scene>,
    cur: Option<Scene>,
    /// When `cur` arrived and how long to take walking from `prev` to it.
    cur_at: f64,
    span: f64,
    /// Events already in the console, so a poll never repeats one.
    seen: HashSet<String>,
}

type Shared = Rc<RefCell<Display>>;

impl Display {
    fn new() -> Result<Display, JsValue> {
        let canvas: HtmlCanvasElement = document()
            .get_element_by_id("screen")
            .ok_or("no #screen canvas")?
            .dyn_into()?;
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")?
            .ok_or("no 2d context")?
            .dyn_into()?;
        let w = canvas.width() as i32;
        let h = canvas.height() as i32;
        Ok(Display {
            ctx,
            frame: Frame::new(w, h),
            prev: None,
            cur: None,
            cur_at: 0.0,
            span: 1000.0,
            seen: HashSet::new(),
        })
    }

    fn set_scene(&mut self, v: &Value) {
        let Some(scene) = Scene::from_json(v) else {
            return;
        };
        let t = now();
        if let Some(cur) = self.cur.take() {
            self.span = (t - self.cur_at).clamp(300.0, 3500.0);
            self.prev = Some(cur);
        }
        self.cur = Some(scene);
        self.cur_at = t;
    }

    fn render(&mut self, t: f64) {
        let Some(cur) = &self.cur else { return };
        let k = ((t - self.cur_at) / self.span).clamp(0.0, 1.0) as f32;
        // The window is a fixed square whatever the world's size.
        let want_w = VIEW;
        let want_h = VIEW;
        if self.frame.w != want_w || self.frame.h != want_h {
            self.frame = Frame::new(want_w, want_h);
            if let Some(canvas) = self.ctx.canvas() {
                canvas.set_width(want_w as u32);
                canvas.set_height(want_h as u32);
            }
        }
        draw::draw(&mut self.frame, self.prev.as_ref(), cur, k, t);
        if let Ok(data) = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(&self.frame.px),
            self.frame.w as u32,
            self.frame.h as u32,
        ) {
            let _ = self.ctx.put_image_data(&data, 0.0, 0.0);
        }
    }
}

fn start_render_loop(display: Shared) {
    let f: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();
    *g.borrow_mut() = Some(Closure::new(move |t: f64| {
        display.borrow_mut().render(t);
        if let Some(cb) = f.borrow().as_ref() {
            let _ = window().request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }));
    // A named borrow: a temporary here would outlive `g` as the tail expression.
    let first = g.borrow();
    if let Some(cb) = first.as_ref() {
        let _ = window().request_animation_frame(cb.as_ref().unchecked_ref());
    }
}

// ---------------------------------------------------------------------------
// The page: header, console
// ---------------------------------------------------------------------------

/// The permanent header: who you are, where, doing what, and what you hold.
fn render_status(v: &Value) {
    let tick = v.get("tick").as_f64().unwrap_or(0.0) as u64;
    let s = v.get("status");
    if s.is_null() {
        set_text("who", "watching");
        set_text("where", &format!("tick {tick}"));
        set_text("doing", "pick a name to play");
        set_text("holding", "");
        set_text("skills", "");
        set_text("recipes", "");
        set_text("script", "");
        return;
    }
    set_text("who", s.get("name").as_str().unwrap_or(""));
    let place = s
        .get("place")
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "({},{})",
                s.get("x").as_i64().unwrap_or(0),
                s.get("y").as_i64().unwrap_or(0)
            )
        });
    set_text("where", &format!("{place} · tick {tick}"));
    let then = s.get("then").to_text();
    let doing = s.get("doing").to_text();
    set_text(
        "doing",
        &if then.is_empty() {
            doing
        } else {
            format!("{doing}, then {then}")
        },
    );
    let list = |key: &str| -> String {
        let items: Vec<String> = s
            .get(key)
            .as_arr()
            .iter()
            .map(|p| format!("{} {}", p.at(1).to_text(), p.at(0).to_text()))
            .collect();
        if items.is_empty() {
            "nothing".to_string()
        } else {
            items.join(", ")
        }
    };
    set_text(
        "holding",
        &format!("carrying {} · bank {}", list("carrying"), list("bank")),
    );
    let skills: Vec<String> = s
        .get("skills")
        .as_arr()
        .iter()
        .map(|p| format!("{} {}", p.at(0).to_text(), p.at(1).to_text()))
        .collect();
    set_text(
        "skills",
        &if skills.is_empty() {
            String::new()
        } else {
            format!("skills: {}", skills.join(", "))
        },
    );
    let recipes: Vec<String> = s
        .get("recipes")
        .as_arr()
        .iter()
        .map(|p| format!("{} = {}", p.at(0).to_text(), p.at(1).to_text()))
        .collect();
    set_text(
        "recipes",
        &if recipes.is_empty() {
            String::new()
        } else {
            format!("recipes: {}", recipes.join("; "))
        },
    );
    set_text(
        "script",
        if s.get("script").as_bool().unwrap_or(false) {
            "a standing script decides what to do when idle"
        } else {
            ""
        },
    );
}

/// One console line, in order, newest at the bottom, scrolled into view.
fn console(kind: &str, tick: Option<u64>, text: &str) {
    let doc = document();
    let Some(el) = doc.get_element_by_id("log") else {
        return;
    };
    let Ok(line) = doc.create_element("div") else {
        return;
    };
    line.set_class_name(&format!("line {kind}"));
    if let Ok(stamp) = doc.create_element("span") {
        stamp.set_class_name("tick");
        stamp.set_text_content(Some(&tick.map(|t| format!("t{t}")).unwrap_or_default()));
        let _ = line.append_child(&stamp);
    }
    if let Ok(body) = doc.create_element("span") {
        body.set_text_content(Some(text));
        let _ = line.append_child(&body);
    }
    let _ = el.append_child(&line);
    while el.child_element_count() > CONSOLE_LINES {
        if let Some(first) = el.first_element_child() {
            first.remove();
        }
    }
    el.set_scroll_top(el.scroll_height());
}

/// Fold a server response into the page: scene, header, and new events.
fn render_view(display: &Shared, v: &Value) {
    let scene = v.get("scene");
    if !scene.is_null() {
        // Paint at once as well: a hidden tab gets no animation frames, and
        // should still show the world the moment it is looked at.
        let mut d = display.borrow_mut();
        d.set_scene(scene);
        let t = now();
        d.render(t);
    }
    render_status(v);
    let mut d = display.borrow_mut();
    for e in v.get("events").as_arr() {
        let tick = e.get("tick").as_f64().unwrap_or(0.0) as u64;
        let name = e.get("name").to_text();
        let text = e.get("text").to_text();
        let kind = e.get("kind").as_str().unwrap_or("note");
        let key = format!("{tick}|{name}|{text}");
        if !d.seen.insert(key) {
            continue;
        }
        let shown = match kind {
            "say" | "voice" => format!(
                "{name}: {}",
                text.strip_prefix("says ")
                    .unwrap_or(&text)
                    .trim_matches('"')
            ),
            _ => format!("{name} {text}"),
        };
        console(kind, Some(tick), &shown);
    }
}

// ---------------------------------------------------------------------------
// Demo: a world in the tab, no server
// ---------------------------------------------------------------------------

fn run_demo(display: Shared) {
    use world::{Command, World};
    fn view(w: &World, me: world::PlayerId) -> Value {
        let events: Vec<Value> = w
            .events
            .iter()
            .rev()
            .take(40)
            .map(|e| obj! {"tick" => e.tick, "name" => e.name.as_str(), "text" => e.text.as_str(), "kind" => e.kind})
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        obj! {"scene" => w.scene(Some(me)), "status" => w.status(me), "tick" => w.tick, "name" => "You", "events" => events}
    }
    let mut w = World::new(7);
    let me = w.join("You");
    let ann = w.join("Ann");
    let _ = w.plan(
        ann,
        vec![
            Command::Gather {
                resource: "wood".into(),
                amount: Some(6),
            },
            Command::Bank,
        ],
    );
    let _ = w.apply(
        ann,
        &Command::SaveRecipe {
            name: "woodrun".into(),
        },
    );
    let _ = w.apply(
        ann,
        &Command::RunRecipe {
            name: "woodrun".into(),
            forever: true,
        },
    );
    let _ = w.apply(
        me,
        &Command::CreateNpc {
            name: "Wren".into(),
            persona: "A forager who talks to birds.".into(),
        },
    );
    let _ = w.apply(
        me,
        &Command::CreateNpc {
            name: "Old Marn".into(),
            persona: "A miner who remembers.".into(),
        },
    );
    let _ = w.apply(
        me,
        &Command::Say {
            text: "Morning, Wren. Off to the hill for iron, then the bank.".into(),
        },
    );
    let _ = w.plan(
        me,
        vec![
            Command::Gather {
                resource: "iron".into(),
                amount: Some(5),
            },
            Command::Bank,
        ],
    );
    let _ = w.apply(
        me,
        &Command::SaveRecipe {
            name: "ironrun".into(),
        },
    );
    let _ = w.apply(
        me,
        &Command::RunRecipe {
            name: "ironrun".into(),
            forever: true,
        },
    );
    show("join", false);
    show("play", false);
    set_text("status", "demo: a local world, no server");
    render_view(&display, &view(&w, me));
    let world = Rc::new(RefCell::new(w));
    let tick = Closure::<dyn FnMut()>::new(move || {
        let mut w = world.borrow_mut();
        w.step();
        let t = w.tick;
        if t % 25 == 0 {
            let _ = w.apply(
                me,
                &Command::Say {
                    text: format!("tick {t} and still going"),
                },
            );
        }
        render_view(&display, &view(&w, me));
    });
    let _ = window().set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        1000,
    );
    tick.forget();
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

async fn main() -> Result<(), JsValue> {
    let display: Shared = Rc::new(RefCell::new(Display::new()?));
    start_render_loop(display.clone());

    let query = window().location().search().unwrap_or_default();
    if query.contains("demo") {
        run_demo(display);
        return Ok(());
    }

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
        let display = display.clone();
        let on_join = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |e: web_sys::KeyboardEvent| {
                if e.key() != "Enter" {
                    return;
                }
                let Some(field) = input("name") else { return };
                let name = field.value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let token = token.clone();
                let display = display.clone();
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
                            render_view(&display, &v);
                            // What to say to a world piloted by words.
                            console(
                                "ack",
                                None,
                                "You're in. Tell your character what to do, in plain words:",
                            );
                            console("ack", None, "chop 10 wood and bank it · build a hut here · make a smith called Brannock");
                            console("ack", None, "hand Nettle 3 fish · forge a lantern from 2 iron · whenever I have 20 wood, bank it");
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
            },
        );
        if let Some(field) = input("name") {
            field.add_event_listener_with_callback("keydown", on_join.as_ref().unchecked_ref())?;
            let _ = field.focus();
        }
        on_join.forget();
    }

    // Speaking: a line, whenever.
    {
        let token = token.clone();
        let display = display.clone();
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
                console("you", None, &format!("> {words}"));
                let token = token.clone();
                let display = display.clone();
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
                                console(
                                    "pilot",
                                    None,
                                    &format!(
                                        "pilot: {pilot}  ({} ms)",
                                        v.get("ms").as_f64().unwrap_or(0.0) as u64
                                    ),
                                );
                            }
                            if let Some(ack) = v.get("ack").as_str() {
                                for line in ack.lines() {
                                    console(
                                        if line.starts_with("x ") { "err" } else { "ack" },
                                        None,
                                        line,
                                    );
                                }
                            }
                            render_view(&display, &v);
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
            Ok((200, v)) => render_view(&display, &v),
            Ok((_, v)) => set_text("status", v.get("error").as_str().unwrap_or("…")),
            Err(e) => set_text("status", &e),
        }
        sleep(POLL_MS).await;
    }
}
