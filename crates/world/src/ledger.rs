//! The ledger: what happened, in order, as data. `state = fold(ledger)`.
//!
//! A `Realm` is a world plus the tokens that name its players and the moment
//! it was last advanced to. Every change to it is an `Entry` — a player joined,
//! a player's words became a plan, an NPC answered — stamped with wall-clock
//! milliseconds. Folding the same entries always yields the same realm, so a
//! ledger is the save file, the replay, and the audit trail at once, and a
//! snapshot (`Realm::to_json`) is only ever an optimisation.
//!
//! Time is entries' timestamps, not a clock: between two entries the world
//! ticks once per second, capped at `GAP_CAP_SECS` — a world nobody is
//! watching sleeps rather than running up a bill of a million ticks.

use gemini::{arr, obj, Value};

use crate::{Command, NpcId, PlayerId, World};

/// The most seconds a world advances across one gap between events.
pub const GAP_CAP_SECS: u64 = 600;
pub const PLAYER_NAME_MAX: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A token claims a character name. Idempotent for the same token.
    Join { token: String, name: String },
    /// A player's plan, as the pilot produced it.
    Plan { token: String, cmds: Vec<Command> },
    /// An NPC's voice answering the speech made to it at `for_tick`.
    NpcSays {
        npc: NpcId,
        for_tick: u64,
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub at_ms: u64,
    pub kind: Kind,
}

impl Entry {
    pub fn to_json(&self) -> Value {
        let mut v = obj! {"at" => self.at_ms};
        match &self.kind {
            Kind::Join { token, name } => {
                v.set("k", "join");
                v.set("token", token.as_str());
                v.set("name", name.as_str());
            }
            Kind::Plan { token, cmds } => {
                v.set("k", "plan");
                v.set("token", token.as_str());
                v.set(
                    "cmds",
                    cmds.iter().map(Command::to_json).collect::<Vec<_>>(),
                );
            }
            Kind::NpcSays {
                npc,
                for_tick,
                text,
            } => {
                v.set("k", "npc");
                v.set("npc", npc.0);
                v.set("tick", *for_tick);
                v.set("text", text.as_str());
            }
        }
        v
    }

    pub fn from_json(v: &Value) -> Result<Entry, String> {
        let at_ms = v.get("at").as_f64().ok_or("entry without a time")? as u64;
        let kind = match v.get("k").as_str() {
            Some("join") => Kind::Join {
                token: v.get("token").to_text(),
                name: v.get("name").to_text(),
            },
            Some("plan") => Kind::Plan {
                token: v.get("token").to_text(),
                cmds: v
                    .get("cmds")
                    .as_arr()
                    .iter()
                    .map(Command::from_json)
                    .collect::<Result<_, _>>()?,
            },
            Some("npc") => Kind::NpcSays {
                npc: NpcId(v.get("npc").as_u32().ok_or("npc entry without an id")?),
                for_tick: v.get("tick").as_f64().unwrap_or(0.0) as u64,
                text: v.get("text").to_text(),
            },
            other => return Err(format!("unknown entry kind {other:?}")),
        };
        Ok(Entry { at_ms, kind })
    }
}

impl Command {
    pub fn to_json(&self) -> Value {
        match self {
            Command::MoveTo { target } => obj! {"c" => "move_to", "target" => target.as_str()},
            Command::Gather { resource, amount } => {
                obj! {"c" => "gather", "resource" => resource.as_str()}.with_opt("amount", *amount)
            }
            Command::Bank => obj! {"c" => "bank"},
            Command::Say { text } => obj! {"c" => "say", "text" => text.as_str()},
            Command::Look => obj! {"c" => "look"},
            Command::Stop => obj! {"c" => "stop"},
            Command::SaveRecipe { name } => obj! {"c" => "save", "name" => name.as_str()},
            Command::RunRecipe { name, forever } => obj! {"c" => "run", "name" => name.as_str(), "forever" => *forever},
            Command::FoundPlace { name, description, resource, skill } => {
                obj! {"c" => "found", "name" => name.as_str(), "description" => description.as_str()}
                    .with_opt("resource", resource.as_deref())
                    .with_opt("skill", skill.as_deref())
            }
            Command::CreateNpc { name, persona } => obj! {"c" => "npc", "name" => name.as_str(), "persona" => persona.as_str()},
        }
    }

    pub fn from_json(v: &Value) -> Result<Command, String> {
        let text = |k: &str| v.get(k).to_text();
        let opt = |k: &str| {
            v.get(k)
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Ok(match v.get("c").as_str() {
            Some("move_to") => Command::MoveTo {
                target: text("target"),
            },
            Some("gather") => Command::Gather {
                resource: text("resource"),
                amount: v.get("amount").as_u32(),
            },
            Some("bank") => Command::Bank,
            Some("say") => Command::Say { text: text("text") },
            Some("look") => Command::Look,
            Some("stop") => Command::Stop,
            Some("save") => Command::SaveRecipe { name: text("name") },
            Some("run") => Command::RunRecipe {
                name: text("name"),
                forever: v.get("forever").as_bool().unwrap_or(false),
            },
            Some("found") => Command::FoundPlace {
                name: text("name"),
                description: text("description"),
                resource: opt("resource"),
                skill: opt("skill"),
            },
            Some("npc") => Command::CreateNpc {
                name: text("name"),
                persona: text("persona"),
            },
            other => return Err(format!("unknown command {other:?}")),
        })
    }
}

/// A world, who its players are, and when it last moved.
#[derive(Clone, Debug, PartialEq)]
pub struct Realm {
    pub world: World,
    /// token → character. Tokens are the temporary identity: a secret the
    /// client keeps, to be replaced by a real login later.
    pub tokens: Vec<(String, PlayerId)>,
    pub last_ms: u64,
}

impl Realm {
    /// A fresh world at `at_ms`, with Ann already at work so it reads as inhabited.
    pub fn genesis(seed: u64, at_ms: u64) -> Realm {
        let mut world = World::new(seed);
        let ann = world.join("Ann");
        let _ = world.plan(
            ann,
            vec![
                Command::Gather {
                    resource: "wood".into(),
                    amount: Some(6),
                },
                Command::Bank,
            ],
        );
        let _ = world.apply(
            ann,
            &Command::SaveRecipe {
                name: "woodrun".into(),
            },
        );
        let _ = world.apply(
            ann,
            &Command::RunRecipe {
                name: "woodrun".into(),
                forever: true,
            },
        );
        Realm {
            world,
            tokens: vec![("bot:ann".into(), ann)],
            last_ms: at_ms,
        }
    }

    pub fn player(&self, token: &str) -> Option<PlayerId> {
        self.tokens
            .iter()
            .find(|(t, _)| t == token)
            .map(|(_, id)| *id)
    }

    /// Tick the world up to `now_ms`, one tick per second, capped per gap.
    pub fn advance_to(&mut self, now_ms: u64) {
        if now_ms <= self.last_ms {
            return;
        }
        let secs = (now_ms - self.last_ms) / 1000;
        if secs > GAP_CAP_SECS {
            for _ in 0..GAP_CAP_SECS {
                self.world.step();
            }
            self.last_ms = now_ms;
        } else {
            for _ in 0..secs {
                self.world.step();
            }
            self.last_ms += secs * 1000;
        }
    }

    /// Advance to the entry's moment, then apply it. `Ok` is the world's
    /// acknowledgement; `Err` is a refusal. Either way the entry counts as
    /// folded — a refused plan is still history.
    pub fn apply(&mut self, e: &Entry) -> Result<String, String> {
        self.advance_to(e.at_ms);
        match &e.kind {
            Kind::Join { token, name } => {
                if let Some(id) = self.player(token) {
                    return Ok(self
                        .world
                        .player(id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default());
                }
                let name = clean_player_name(name)?;
                if self
                    .world
                    .players
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(&name))
                {
                    return Err(format!("the name {name} is taken"));
                }
                let id = self.world.join(name.clone());
                self.tokens.push((token.clone(), id));
                Ok(name)
            }
            Kind::Plan { token, cmds } => {
                let id = self.player(token).ok_or("unknown player")?;
                self.world.plan(id, cmds.clone())
            }
            Kind::NpcSays {
                npc,
                for_tick,
                text,
            } => {
                self.world.npc_says(*npc, text);
                self.world.answer_speech(*npc, *for_tick);
                Ok(String::new())
            }
        }
    }

    /// Fold a ledger from genesis. The world is born when its first entry is.
    pub fn fold(seed: u64, entries: &[Entry], now_ms: u64) -> Realm {
        let born = entries.first().map(|e| e.at_ms).unwrap_or(now_ms);
        let mut realm = Realm::genesis(seed, born);
        for e in entries {
            let _ = realm.apply(e);
        }
        realm.advance_to(now_ms);
        realm
    }

    pub fn to_json(&self) -> Value {
        obj! {
            "last_ms" => self.last_ms,
            "tokens" => self.tokens.iter().map(|(t, id)| arr![t.as_str(), id.0]).collect::<Vec<_>>(),
            "world" => self.world.to_json(),
        }
    }

    pub fn from_json(v: &Value) -> Result<Realm, String> {
        Ok(Realm {
            world: World::from_json(v.get("world"))?,
            tokens: v
                .get("tokens")
                .as_arr()
                .iter()
                .map(|t| {
                    Ok((
                        t.at(0).to_text(),
                        PlayerId(t.at(1).as_u32().ok_or("bad token id")?),
                    ))
                })
                .collect::<Result<_, String>>()?,
            last_ms: v.get("last_ms").as_f64().unwrap_or(0.0) as u64,
        })
    }
}

fn clean_player_name(s: &str) -> Result<String, String> {
    let s: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(PLAYER_NAME_MAX)
        .collect();
    if s.chars().count() < 2 {
        return Err("a name needs at least two characters".into());
    }
    if !s
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '\'' | '-' | '_'))
    {
        return Err(
            "a name is letters, digits, spaces, apostrophes, hyphens or underscores".into(),
        );
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(token: &str, at: u64, cmds: Vec<Command>) -> Entry {
        Entry {
            at_ms: at,
            kind: Kind::Plan {
                token: token.into(),
                cmds,
            },
        }
    }
    fn join(token: &str, at: u64, name: &str) -> Entry {
        Entry {
            at_ms: at,
            kind: Kind::Join {
                token: token.into(),
                name: name.into(),
            },
        }
    }

    #[test]
    fn commands_and_entries_round_trip() {
        let cmds = vec![
            Command::MoveTo {
                target: "Old Forest".into(),
            },
            Command::Gather {
                resource: "wood".into(),
                amount: Some(10),
            },
            Command::Gather {
                resource: "fish".into(),
                amount: None,
            },
            Command::Bank,
            Command::Say {
                text: "hi \"there\"".into(),
            },
            Command::Look,
            Command::Stop,
            Command::SaveRecipe {
                name: "woodrun".into(),
            },
            Command::RunRecipe {
                name: "woodrun".into(),
                forever: true,
            },
            Command::FoundPlace {
                name: "Damp Hollow".into(),
                description: "d".into(),
                resource: Some("mushrooms".into()),
                skill: None,
            },
            Command::CreateNpc {
                name: "Wren".into(),
                persona: "p".into(),
            },
        ];
        for c in &cmds {
            assert_eq!(&Command::from_json(&c.to_json()).unwrap(), c);
        }
        let entries = vec![
            join("t1", 1000, "Kyle"),
            plan("t1", 2000, cmds),
            Entry {
                at_ms: 3000,
                kind: Kind::NpcSays {
                    npc: NpcId(1),
                    for_tick: 7,
                    text: "yes".into(),
                },
            },
        ];
        for e in &entries {
            let text = e.to_json().to_string();
            assert_eq!(&Entry::from_json(&Value::parse(&text).unwrap()).unwrap(), e);
        }
    }

    #[test]
    fn folding_is_deterministic_and_time_is_capped() {
        let entries = vec![
            join("t1", 10_000, "Kyle"),
            plan(
                "t1",
                12_000,
                vec![
                    Command::Gather {
                        resource: "iron".into(),
                        amount: Some(3),
                    },
                    Command::Bank,
                ],
            ),
            join("t2", 15_000, "Bea"),
            plan(
                "t2",
                15_500,
                vec![Command::Say {
                    text: "hello".into(),
                }],
            ),
        ];
        let a = Realm::fold(7, &entries, 100_000);
        let b = Realm::fold(7, &entries, 100_000);
        assert_eq!(a, b);
        // 88 seconds after the last entry: 88 ticks, not more.
        assert_eq!(a.world.tick, 90);
        let kyle = a.player("t1").unwrap();
        assert!(a
            .world
            .player(kyle)
            .unwrap()
            .bank
            .iter()
            .any(|(r, n)| r == "iron" && *n == 3));
        // A year later, the world slept: only the cap's worth of ticks passed.
        let c = Realm::fold(7, &entries, 100_000 + 365 * 86_400_000);
        assert_eq!(c.world.tick, 5 + GAP_CAP_SECS);
        // Sub-second remainders are not lost between frequent observations.
        let mut d = Realm::fold(7, &entries, 15_500);
        for t in 0..10 {
            d.advance_to(15_500 + t * 700);
        }
        assert_eq!(d.world.tick, 5 + 6);
    }

    #[test]
    fn joins_are_idempotent_and_names_unique() {
        let mut r = Realm::genesis(7, 0);
        assert_eq!(r.apply(&join("t1", 0, "Kyle")).unwrap(), "Kyle");
        assert_eq!(r.apply(&join("t1", 0, "Kyle")).unwrap(), "Kyle");
        assert!(r
            .apply(&join("t2", 0, "kyle"))
            .unwrap_err()
            .contains("taken"));
        assert!(r
            .apply(&join("t2", 0, "Ann"))
            .unwrap_err()
            .contains("taken"));
        assert!(r.apply(&join("t3", 0, "x")).is_err());
        assert!(r.apply(&plan("nobody", 0, vec![Command::Look])).is_err());
        assert_eq!(r.world.players.len(), 2);
    }

    #[test]
    fn realm_snapshot_round_trips_mid_plan() {
        let entries = vec![
            join("t1", 1000, "Kyle"),
            plan(
                "t1",
                2000,
                vec![Command::CreateNpc {
                    name: "Wren".into(),
                    persona: "A forager who talks to birds.".into(),
                }],
            ),
            plan(
                "t1",
                3000,
                vec![
                    Command::MoveTo {
                        target: "east".into(),
                    },
                    Command::Say {
                        text: "hi Wren".into(),
                    },
                ],
            ),
        ];
        let r = Realm::fold(7, &entries, 4000);
        let text = r.to_json().to_string();
        let back = Realm::from_json(&Value::parse(&text).unwrap()).unwrap();
        assert_eq!(r, back);
        // Keep folding both: they stay identical.
        let mut r2 = r.clone();
        let mut back2 = back;
        let more = plan(
            "t1",
            20_000,
            vec![Command::Gather {
                resource: "wood".into(),
                amount: Some(2),
            }],
        );
        r2.apply(&more).unwrap();
        back2.apply(&more).unwrap();
        r2.advance_to(60_000);
        back2.advance_to(60_000);
        assert_eq!(r2, back2);
        assert!(r2
            .world
            .player(r2.player("t1").unwrap())
            .unwrap()
            .bank
            .is_empty());
    }
}
