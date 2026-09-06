//! The host: one realm behind a ledger, served one request at a time.
//!
//! Nothing here is long-running. A request loads the latest snapshot and the
//! entries after it, folds them, advances the world to now, does its work —
//! joins a player, pilots their words into a plan, runs idle players' Lua
//! scripts, lets NPCs answer — and appends what happened to the ledger. If
//! nobody else wrote in the meantime, it also stores a fresh snapshot so the
//! next request folds less. Any snapshot is a valid prefix, so a stale one
//! costs replay, never correctness; an unreadable one is replayed around.
//!
//! The store is behind `Ledger`; `Memory` implements it for tests and
//! `neon::Neon` over HTTPS for the deployment.

pub mod neon;

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use gemini::native::Client;
use gemini::{obj, Value};
use world::ledger::{Entry, Kind, Realm};
use world::{pilot, Command, NpcId, PlayerId};

pub const TOKEN_MIN: usize = 8;
pub const TOKEN_MAX: usize = 64;
/// Voices answered per request, so a chatty crowd cannot stall a request.
const VOICES_PER_REQUEST: usize = 2;
/// Scripts run per request.
const SCRIPTS_PER_REQUEST: usize = 6;

/// Where entries go and come from. Ids are the store's own ordering.
pub trait Ledger: Send + Sync {
    /// The latest snapshot — (last entry id it covers, realm JSON) — and every
    /// entry after it, in order, as (id, entry).
    fn load(&self) -> Result<(Option<(u64, Value)>, Vec<(u64, Entry)>), String>;
    /// Append one entry; returns its id.
    fn append(&self, e: &Entry) -> Result<u64, String>;
    /// Store a snapshot covering entries up to and including `last_id`.
    fn snapshot(&self, last_id: u64, realm: &Value) -> Result<(), String>;
    /// Every entry ever, in order, with ids: the whole history, for replays and audits.
    fn all(&self) -> Result<Vec<(u64, Entry)>, String>;
    /// Every entry after `id`, in order.
    fn since(&self, id: u64) -> Result<Vec<(u64, Entry)>, String>;
}

/// An in-memory ledger, for tests and for a single-process server.
#[derive(Default)]
pub struct Memory {
    inner: Mutex<(Vec<Entry>, Option<(u64, Value)>)>,
}

impl Ledger for Memory {
    fn load(&self) -> Result<(Option<(u64, Value)>, Vec<(u64, Entry)>), String> {
        let g = self.inner.lock().unwrap();
        let from = g.1.as_ref().map(|(id, _)| *id).unwrap_or(0);
        let tail =
            g.0.iter()
                .enumerate()
                .map(|(i, e)| (i as u64 + 1, e.clone()))
                .filter(|(id, _)| *id > from)
                .collect();
        Ok((g.1.clone(), tail))
    }
    fn append(&self, e: &Entry) -> Result<u64, String> {
        let mut g = self.inner.lock().unwrap();
        g.0.push(e.clone());
        Ok(g.0.len() as u64)
    }
    fn snapshot(&self, last_id: u64, realm: &Value) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap();
        if g.1.as_ref().map(|(id, _)| *id < last_id).unwrap_or(true) {
            g.1 = Some((last_id, realm.clone()));
        }
        Ok(())
    }
    fn all(&self) -> Result<Vec<(u64, Entry)>, String> {
        let g = self.inner.lock().unwrap();
        Ok(g.0
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect())
    }
    fn since(&self, id: u64) -> Result<Vec<(u64, Entry)>, String> {
        Ok(self.all()?.into_iter().filter(|(i, _)| *i > id).collect())
    }
}

pub struct Reply {
    pub status: u16,
    pub body: Value,
}

impl Reply {
    fn bad(status: u16, why: impl Into<String>) -> Reply {
        Reply {
            status,
            body: obj! {"error" => why.into()},
        }
    }
}

pub struct Host {
    pub ledger: Box<dyn Ledger>,
    pub gemini: Option<Client>,
    pub model: String,
    pub seed: u64,
}

/// An error body, for hosts that failed to start.
pub fn error_body(e: &str) -> Value {
    obj! {"error" => e}
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The API, for people and agents arriving cold: `GET /api/world?doc`.
pub const DOC: &str = "\
cqs — a shared world piloted by words. https://cqs.gg

GET  /api/world?token=T       the world as T sees it (no token: a spectator)
POST /api/world               body: {\"token\": T, \"name\": N?, \"words\": W?, \"cmds\": [..]?, \"script\": S?}
  token   8-64 chars of [A-Za-z0-9_-], your secret; the same token is the same character
  name    joins as N the first time a token is seen (2-16 chars)
  words   what your character should do or say; a model turns it into steps
  cmds    steps directly, no model: [{\"c\":\"move_to\",\"target\":\"Old Forest\"},{\"c\":\"gather\",\"resource\":\"wood\",\"amount\":10},{\"c\":\"bank\"}]
          also {\"c\":\"say\",\"text\"} {\"c\":\"look\"} {\"c\":\"stop\"} {\"c\":\"save\",\"name\"} {\"c\":\"run\",\"name\",\"forever\"}
          {\"c\":\"found\",\"name\",\"description\",\"form\"?,\"style\"?,\"resource\"?,\"skill\"?} {\"c\":\"build\",\"site\"} {\"c\":\"abandon\",\"site\"}
          {\"c\":\"give\",\"item\",\"amount\"?,\"to\"} {\"c\":\"want\",\"npc\",\"item\",\"amount\",\"reward\":[[\"gold\",2]],\"repeat\",\"words\"}
          {\"c\":\"craft\",\"item\",\"description\",\"from\":[[\"iron\",2]]} (at a built building, from carried materials)
          {\"c\":\"offer\",\"item\",\"amount\",\"reward\":[[\"gold\",1]],\"repeat\",\"words\"} (a shop: what you buy from other players and pay from your pack; amount 0 withdraws)
          {\"c\":\"npc_script\",\"npc\",\"source\"} (a standing Lua script for a character you made; walk(\"home\") returns them) {\"c\":\"npc\",\"name\",\"persona\"} {\"c\":\"script\",\"source\"}
          form: banner (a free spot) or a building — hut, house, hall, tower, spire, forge, mill, shrine, well — marked out with a bill of
          materials that must be CARRIED to the site; build walks there, hands over what is carried, and works until it stands
  script  a standing Lua script (empty string clears it); runs whenever your character is idle, at most once every five ticks

Reply: {tick, view (text, the same the pilot reads), status {name, place, doing, then, carrying, bank, skills, recipes, script},
        scene {w, h, tiles, places, npcs, players, speech}, events [{tick, name, text, kind}], players, ack?, pilot?, ms?}

Scripts see: me {name, x, y, place, doing, carrying, bank, skills}, places [{name, x, y, resource, distance}],
  people [{name, x, y, npc, distance}], tick, memory (persists between runs).
Scripts call: walk(target) gather(resource, amount?) bank() say(text) stop() found(name, description, resource?, skill?)
  npc(name, persona) near(name) dist(name) log(text). A run gets 200k instructions and may issue 8 steps.

The world ticks once a second while anyone is watching; steps run one after another; gathering trains a skill.
Founded places and created people are for everyone. Nobody stands on anybody.
";

impl Host {
    /// Everything from the environment: `DATABASE_URL` (Neon), `GEMINI_API_KEY`,
    /// `GEMINI_MODEL`, `CQS_SEED`.
    pub fn from_env() -> Result<Host, String> {
        gemini::native::dotenv();
        let ledger: Box<dyn Ledger> = match neon::Neon::from_env() {
            Some(n) => Box::new(n),
            None => return Err("DATABASE_URL is not set".into()),
        };
        Ok(Host {
            ledger,
            gemini: Client::from_env().ok(),
            model: std::env::var("GEMINI_MODEL")
                .unwrap_or_else(|_| pilot::DEFAULT_MODEL.to_string()),
            seed: std::env::var("CQS_SEED")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(7),
        })
    }

    /// Route one HTTP request. `query` is the raw query string, `body` the raw body.
    pub fn handle(&self, method: &str, query: &str, body: &str) -> Reply {
        let now = now_ms();
        match method {
            "GET" => {
                if param(query, "doc").is_some() || query.split('&').any(|k| k == "doc") {
                    return Reply {
                        status: 200,
                        body: obj! {"doc" => DOC},
                    };
                }
                let token = param(query, "token");
                self.get(token.as_deref(), now)
            }
            "POST" => match Value::parse(body) {
                Ok(v) => self.post(&v, now),
                Err(e) => Reply::bad(400, format!("bad json: {e}")),
            },
            _ => Reply::bad(405, "GET or POST"),
        }
    }

    /// Fold the ledger and bring the realm to `now`. Returns the realm and the
    /// id of the last entry folded (0 when the ledger is empty).
    fn load(&self, now: u64) -> Result<(Realm, u64), String> {
        let (snap, mut tail) = self.ledger.load()?;
        let snapshot = match snap {
            Some((id, json)) => match Realm::from_json(&json) {
                Ok(r) => Some((r, id)),
                Err(_) => {
                    // A snapshot this build cannot read (the world's shape
                    // changed): the ledger is the truth, replay all of it.
                    tail = self.ledger.all()?;
                    None
                }
            },
            None => None,
        };
        let (mut realm, mut last_id) = match snapshot {
            Some(s) => s,
            None => (
                Realm::genesis(self.seed, tail.first().map(|(_, e)| e.at_ms).unwrap_or(now)),
                0,
            ),
        };
        for (id, e) in &tail {
            let _ = realm.apply(e);
            last_id = *id;
        }
        realm.advance_to(now);
        Ok((realm, last_id))
    }

    /// Append an entry and apply it. Returns the id and the world's answer.
    fn commit(
        &self,
        realm: &mut Realm,
        e: &Entry,
    ) -> Result<(u64, Result<String, String>), String> {
        let id = self.ledger.append(e)?;
        let ack = realm.apply(e);
        Ok((id, ack))
    }

    /// Store a snapshot if this request saw every entry up to the one it wrote.
    fn maybe_snapshot(&self, realm: &Realm, loaded_last: u64, written: &[u64]) {
        let Some(&last) = written.last() else { return };
        let contiguous = written.first() == Some(&(loaded_last + 1))
            && written.windows(2).all(|w| w[1] == w[0] + 1);
        if contiguous {
            let _ = self.ledger.snapshot(last, &realm.to_json());
        }
    }

    /// Let NPCs answer what was said to them. Each answer is a ledger entry.
    /// Two requests can see the same unanswered speech, so the ledger is
    /// checked for an answer that landed since we loaded.
    fn voices(
        &self,
        realm: &mut Realm,
        now: u64,
        loaded_last: u64,
        written: &mut Vec<u64>,
        limit: usize,
    ) -> Vec<String> {
        let Some(client) = &self.gemini else {
            return Vec::new();
        };
        let pending: Vec<_> = realm.world.speeches().iter().take(limit).cloned().collect();
        if pending.is_empty() {
            return Vec::new();
        }
        let answered: Vec<(NpcId, u64)> = self
            .ledger
            .since(loaded_last)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, e)| match e.kind {
                Kind::NpcSays { npc, for_tick, .. } => Some((npc, for_tick)),
                _ => None,
            })
            .collect();
        let mut lines = Vec::new();
        for s in pending {
            if answered.contains(&(s.listener, s.tick)) {
                realm.world.answer_speech(s.listener, s.tick);
                continue;
            }
            let Some(npc) = realm.world.npc(s.listener).cloned() else {
                continue;
            };
            let speaker = realm
                .world
                .player(s.speaker)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let view = realm.world.describe(s.speaker);
            let reply =
                match client.generate(&pilot::voice(&self.model, &npc, &view, &speaker, &s.text)) {
                    Ok(r) => r.text().trim().to_string(),
                    Err(_) => String::new(),
                };
            if reply.is_empty() {
                continue;
            }
            let e = Entry {
                at_ms: now,
                kind: Kind::NpcSays {
                    npc: npc.id,
                    for_tick: s.tick,
                    text: reply.clone(),
                },
            };
            if let Ok((id, _)) = self.commit(realm, &e) {
                written.push(id);
                lines.push(format!("{} says \"{reply}\"", npc.name));
            }
        }
        lines
    }

    /// Run the standing scripts of idle players. What a script decides goes
    /// into the ledger as a `Ran` entry, so a replay never runs Lua.
    fn scripts(&self, realm: &mut Realm, now: u64, written: &mut Vec<u64>) -> Vec<String> {
        let mut lines = Vec::new();
        for pid in realm
            .world
            .scripted_idle()
            .into_iter()
            .take(SCRIPTS_PER_REQUEST)
        {
            let (source, memory, name) = {
                let Some(p) = realm.world.player(pid) else {
                    continue;
                };
                let Some(src) = p.script.clone() else {
                    continue;
                };
                (src, p.memory.clone(), p.name.clone())
            };
            let Some(token) = realm
                .tokens
                .iter()
                .find(|(_, id)| *id == pid)
                .map(|(t, _)| t.clone())
            else {
                continue;
            };
            let status = realm.world.status(pid);
            let scene = realm.world.scene(Some(pid));
            let out = script::run(&source, &status, &scene, &memory);
            let note = match (&out.error, out.log.is_empty()) {
                (Some(e), _) => format!("script error: {e}"),
                (None, false) => out.log.join(" · "),
                _ => String::new(),
            };
            if out.cmds.is_empty() && out.memory == memory && note.is_empty() {
                // Nothing to record; the script simply chose to wait.
                continue;
            }
            let e = Entry {
                at_ms: now,
                kind: Kind::Ran {
                    token,
                    cmds: out.cmds.clone(),
                    memory: out.memory,
                    note: note.clone(),
                },
            };
            if let Ok((id, ack)) = self.commit(realm, &e) {
                written.push(id);
                let steps: Vec<String> = out.cmds.iter().map(|c| c.to_string()).collect();
                lines.push(format!(
                    "{name}'s script: {}{}",
                    if steps.is_empty() {
                        note.clone()
                    } else {
                        steps.join(" → ")
                    },
                    match ack {
                        Ok(a) if !a.is_empty() => format!(" — {a}"),
                        Err(e) => format!(" — x {e}"),
                        _ => String::new(),
                    }
                ));
            }
        }
        // NPCs with scripts of their own.
        for nid in realm
            .world
            .npc_scripted_idle()
            .into_iter()
            .take(SCRIPTS_PER_REQUEST)
        {
            let Some(n) = realm.world.npc(nid).cloned() else {
                continue;
            };
            let Some(src) = n.script.clone() else {
                continue;
            };
            let status = realm.world.npc_status(nid);
            let scene = realm.world.scene(None);
            let out = script::run(&src, &status, &scene, &n.memory);
            let note = match (&out.error, out.log.is_empty()) {
                (Some(e), _) => format!("script error: {e}"),
                (None, false) => out.log.join(" · "),
                _ => String::new(),
            };
            if out.cmds.is_empty() && out.memory == n.memory && note.is_empty() {
                continue;
            }
            let e = Entry {
                at_ms: now,
                kind: Kind::NpcRan {
                    npc: nid,
                    cmds: out.cmds.clone(),
                    memory: out.memory,
                    note: note.clone(),
                },
            };
            if let Ok((id, ack)) = self.commit(realm, &e) {
                written.push(id);
                let steps: Vec<String> = out.cmds.iter().map(|c| c.to_string()).collect();
                lines.push(format!(
                    "{}'s script: {}{}",
                    n.name,
                    if steps.is_empty() {
                        note.clone()
                    } else {
                        steps.join(" → ")
                    },
                    match ack {
                        Ok(a) if !a.is_empty() => format!(" — {a}"),
                        Err(e) => format!(" — x {e}"),
                        _ => String::new(),
                    }
                ));
            }
        }
        lines
    }

    pub fn get(&self, token: Option<&str>, now: u64) -> Reply {
        let (mut realm, last) = match self.load(now) {
            Ok(x) => x,
            Err(e) => return Reply::bad(500, e),
        };
        let me = token.and_then(|t| realm.player(t));
        let mut written = Vec::new();
        // A poll carries one pending voice and runs due scripts, so the world
        // keeps answering and acting for whoever is watching.
        let mut said = self.voices(&mut realm, now, last, &mut written, 1);
        said.extend(self.scripts(&mut realm, now, &mut written));
        self.maybe_snapshot(&realm, last, &written);
        let mut body = view_json(&realm, me);
        if !said.is_empty() {
            body.set("said", said);
        }
        Reply { status: 200, body }
    }

    pub fn post(&self, req: &Value, now: u64) -> Reply {
        let token = req.get("token").to_text();
        let n = token.chars().count();
        if n < TOKEN_MIN
            || n > TOKEN_MAX
            || !token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Reply::bad(400, "token must be 8-64 letters, digits, - or _");
        }
        let name = req
            .get("name")
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let words = req.get("words").to_text();
        let words = words.trim();
        let direct: Option<Vec<Command>> = if req.get("cmds").is_null() {
            None
        } else {
            match req
                .get("cmds")
                .as_arr()
                .iter()
                .map(Command::from_json)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(c) if !c.is_empty() => Some(c),
                Ok(_) => return Reply::bad(400, "cmds is empty"),
                Err(e) => return Reply::bad(400, format!("bad command: {e}")),
            }
        };
        let script = req.get("script").as_str().map(str::to_string);

        let (mut realm, last) = match self.load(now) {
            Ok(x) => x,
            Err(e) => return Reply::bad(500, e),
        };
        let mut written = Vec::new();
        let mut acks = Vec::new();

        let me = match realm.player(&token) {
            Some(id) => id,
            None => {
                let Some(name) = name else {
                    return Reply::bad(401, "new here? send a name to join");
                };
                let join = Entry {
                    at_ms: now,
                    kind: Kind::Join {
                        token: token.clone(),
                        name: name.to_string(),
                    },
                };
                // Refusals never reach the ledger.
                if let Err(e) = realm.clone().apply(&join) {
                    return Reply::bad(400, e);
                }
                match self.commit(&mut realm, &join) {
                    Ok((id, _)) => written.push(id),
                    Err(e) => return Reply::bad(500, e),
                }
                let id = realm.player(&token).unwrap();
                acks.push(format!(
                    "{} arrives in Town.",
                    realm.world.player(id).unwrap().name
                ));
                id
            }
        };

        let mut piloted = String::new();
        let mut ms = 0u128;
        // What to do, in order of directness: a script to set, steps given
        // outright, or words for the pilot.
        let mut plans: Vec<Vec<Command>> = Vec::new();
        if let Some(source) = script {
            plans.push(vec![Command::SetScript { source }]);
        }
        if let Some(cmds) = direct {
            plans.push(cmds);
        }
        if !words.is_empty() {
            let words: String = words.chars().take(500).collect();
            let view = realm.world.describe(me);
            let t0 = std::time::Instant::now();
            let cmds: Vec<Command> = match &self.gemini {
                Some(client) => {
                    match client.generate(&pilot::request(&self.model, &view, &words)) {
                        Ok(resp) => {
                            let mut cmds: Vec<Command> = pilot::commands(&resp)
                                .into_iter()
                                .filter_map(Result::ok)
                                .collect();
                            if cmds.is_empty() {
                                let text = resp.text();
                                cmds = if text.trim().is_empty() {
                                    pilot::guess(&words)
                                } else {
                                    vec![Command::Say { text }]
                                };
                            }
                            cmds
                        }
                        Err(_) => pilot::guess(&words),
                    }
                }
                None => pilot::guess(&words),
            };
            ms = t0.elapsed().as_millis();
            piloted = cmds
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" → ");
            plans.push(cmds);
        }
        for cmds in plans {
            let plan = Entry {
                at_ms: now,
                kind: Kind::Plan {
                    token: token.clone(),
                    cmds,
                },
            };
            match self.commit(&mut realm, &plan) {
                Ok((id, ack)) => {
                    written.push(id);
                    acks.push(match ack {
                        Ok(a) => a,
                        Err(e) => format!("x {e}"),
                    });
                }
                Err(e) => return Reply::bad(500, e),
            }
        }

        let mut said = self.voices(&mut realm, now, last, &mut written, VOICES_PER_REQUEST);
        said.extend(self.scripts(&mut realm, now, &mut written));
        self.maybe_snapshot(&realm, last, &written);

        let mut body = view_json(&realm, Some(me));
        body.set("ack", acks.join("\n"));
        body.set("pilot", piloted);
        body.set("ms", ms as u64);
        if !said.is_empty() {
            body.set("said", said);
        }
        Reply { status: 200, body }
    }
}

fn view_json(realm: &Realm, me: Option<PlayerId>) -> Value {
    let w = &realm.world;
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
    let mut v = obj! {
        "tick" => w.tick,
        "map" => w.ascii(),
        "scene" => w.scene(me),
        "events" => events,
        "players" => w.players.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
    };
    match me.and_then(|id| w.player(id)) {
        Some(p) => {
            v.set("name", p.name.as_str());
            v.set("view", w.describe(p.id));
            v.set("status", w.status(p.id));
        }
        None => {
            let recent: Vec<String> = w
                .events
                .iter()
                .rev()
                .take(8)
                .map(|e| format!("[t{}] {} {}", e.tick, e.name, e.text))
                .collect();
            v.set(
                "view",
                format!(
                    "You are watching. Recently: {}",
                    recent.into_iter().rev().collect::<Vec<_>>().join("; ")
                ),
            );
        }
    }
    v
}

/// One query-string parameter, percent-decoded.
pub fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(
                    std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                    16,
                ) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            ledger: Box::new(Memory::default()),
            gemini: None,
            model: "none".into(),
            seed: 7,
        }
    }

    #[test]
    fn join_prompt_and_watch_the_world_move() {
        let h = host();
        let t = 1_000_000u64;
        // A stranger must give a name.
        let r = h.post(&obj! {"token" => "abcdefgh", "words" => "look"}, t);
        assert_eq!(r.status, 401);
        let r = h.post(
            &obj! {"token" => "abcdefgh", "name" => "Bea", "words" => "chop 3 wood then bank it"},
            t,
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert_eq!(r.body.get("name").as_str(), Some("Bea"));
        assert!(r
            .body
            .get("pilot")
            .as_str()
            .unwrap()
            .contains("gather 3 wood"));
        assert!(r
            .body
            .get("ack")
            .as_str()
            .unwrap()
            .contains("heads for Old Forest"));
        assert_eq!(r.body.get("status").get("script").as_bool(), Some(false));
        // Same name, other token: refused, nothing written.
        let r = h.post(&obj! {"token" => "zzzzzzzz", "name" => "bea"}, t + 10);
        assert_eq!(r.status, 400);
        // Two minutes later the plan has run: the wood is in the bank.
        let r = h.get(Some("abcdefgh"), t + 120_000);
        let view = r.body.get("view").as_str().unwrap();
        assert!(view.contains("Bank: 3 wood"), "{view}");
        assert!(r.body.get("map").as_str().unwrap().contains('B'));
        assert!(!r.body.get("events").as_arr().is_empty());
        // A spectator sees the map and the log, not a character.
        let r = h.get(None, t + 120_000);
        assert!(r
            .body
            .get("view")
            .as_str()
            .unwrap()
            .starts_with("You are watching"));
        assert_eq!(r.body.get("players").as_arr().len(), 2);
        // Bad tokens are refused before anything is read.
        assert_eq!(
            h.post(&obj! {"token" => "short", "name" => "X"}, t).status,
            400
        );
        assert_eq!(h.handle("PUT", "", "").status, 405);
        assert_eq!(h.handle("POST", "", "not json").status, 400);
        assert!(h
            .handle("GET", "doc", "")
            .body
            .get("doc")
            .as_str()
            .unwrap()
            .contains("POST /api/world"));
    }

    #[test]
    fn direct_commands_need_no_pilot() {
        let h = host();
        let t = 2_000_000u64;
        let r = h.post(
            &obj! {"token" => "agent-0001", "name" => "Bot", "cmds" => vec![
                obj! {"c" => "gather", "resource" => "iron", "amount" => 2},
                obj! {"c" => "bank"},
            ]},
            t,
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert!(r
            .body
            .get("ack")
            .as_str()
            .unwrap()
            .contains("heads for Iron Hill"));
        assert_eq!(r.body.get("pilot").as_str(), Some(""));
        let r = h.get(Some("agent-0001"), t + 90_000);
        assert!(r
            .body
            .get("view")
            .as_str()
            .unwrap()
            .contains("Bank: 2 iron"));
        assert_eq!(
            h.post(
                &obj! {"token" => "agent-0001", "cmds" => vec![obj! {"c" => "fly"}]},
                t
            )
            .status,
            400
        );
    }

    #[test]
    fn a_standing_script_runs_when_idle_and_is_recorded() {
        let h = host();
        let t = 3_000_000u64;
        let src = "memory.n = (memory.n or 0) + 1\nlog('run ' .. memory.n)\nif (me.bank.wood or 0) < 2 then gather('wood', 1) bank() else say('enough wood') end";
        let r = h.post(
            &obj! {"token" => "abcdefgh", "name" => "Bea", "script" => src},
            t,
        );
        assert_eq!(r.status, 200, "{}", r.body);
        assert!(r
            .body
            .get("ack")
            .as_str()
            .unwrap()
            .contains("sets a script"));
        assert_eq!(r.body.get("status").get("script").as_bool(), Some(true));
        // The script ran at once (the character was idle) and chose to gather.
        let said = r.body.get("said").as_arr();
        assert!(
            said.iter()
                .any(|s| s.as_str().unwrap().contains("gather 1 wood")),
            "{}",
            r.body
        );
        // Polls keep it going: two rounds of wood, then it speaks.
        let mut spoke = false;
        for i in 1..=8 {
            let r = h.get(Some("abcdefgh"), t + i * 40_000);
            if r.body.get("view").as_str().unwrap().contains("enough wood") {
                spoke = true;
                break;
            }
        }
        assert!(spoke, "the script never reached its goal");
        // Its memory and runs are in the ledger, so a fresh fold agrees.
        let all: Vec<Entry> = h
            .ledger
            .all()
            .unwrap()
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        assert!(all
            .iter()
            .any(|e| matches!(&e.kind, Kind::Ran { note, .. } if note.starts_with("run "))));
        let folded = Realm::fold(7, &all, t + 9 * 40_000);
        let bea = folded.player("abcdefgh").unwrap();
        assert!(
            folded
                .world
                .player(bea)
                .unwrap()
                .memory
                .get("n")
                .as_i64()
                .unwrap()
                >= 2
        );
        // Clearing.
        let r = h.post(&obj! {"token" => "abcdefgh", "script" => ""}, t + 400_000);
        assert!(r
            .body
            .get("ack")
            .as_str()
            .unwrap()
            .contains("clears the script"));
        assert_eq!(r.body.get("status").get("script").as_bool(), Some(false));
    }

    #[test]
    fn snapshots_advance_only_over_contiguous_writes_and_fold_the_same() {
        let h = host();
        let t = 5_000_000u64;
        h.post(
            &obj! {"token" => "abcdefgh", "name" => "Bea", "words" => "go north"},
            t,
        );
        let (snap, tail) = h.ledger.load().unwrap();
        assert_eq!(snap.as_ref().map(|s| s.0), Some(2));
        assert!(tail.is_empty());
        let (realm, last) = h.load(t + 1000).unwrap();
        assert_eq!(last, 2);
        // Someone else's entry slipped in between load and append: no snapshot.
        h.maybe_snapshot(&realm, last, &[4]);
        assert_eq!(h.ledger.load().unwrap().0.map(|s| s.0), Some(2));
        // A gap inside this request's own writes: no snapshot either.
        h.maybe_snapshot(&realm, last, &[3, 5]);
        assert_eq!(h.ledger.load().unwrap().0.map(|s| s.0), Some(2));
        // Contiguous from what was loaded: the snapshot moves forward.
        h.maybe_snapshot(&realm, last, &[3, 4]);
        assert_eq!(h.ledger.load().unwrap().0.map(|s| s.0), Some(4));
        // And never backward.
        h.maybe_snapshot(&realm, 2, &[3]);
        assert_eq!(h.ledger.load().unwrap().0.map(|s| s.0), Some(4));
        // From the snapshot or from scratch, the fold agrees.
        let h2 = host();
        h2.post(
            &obj! {"token" => "abcdefgh", "name" => "Bea", "words" => "go north"},
            t,
        );
        h2.post(
            &obj! {"token" => "abcdefgh", "words" => "chop 2 wood"},
            t + 5000,
        );
        let with = h2.load(t + 60_000).unwrap().0;
        let all: Vec<Entry> = h2
            .ledger
            .all()
            .unwrap()
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        let without = Realm::fold(7, &all, t + 60_000);
        assert_eq!(with, without);
        // A snapshot this code cannot read is replayed around, not tripped over.
        h2.ledger
            .snapshot(99, &obj! {"world" => "not a world"})
            .unwrap();
        assert_eq!(h2.load(t + 60_000).unwrap().0, without);
    }

    #[test]
    fn query_params_decode() {
        assert_eq!(
            param("a=1&token=ab%20cd+e", "token").as_deref(),
            Some("ab cd e")
        );
        assert_eq!(param("a=1", "token"), None);
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}
