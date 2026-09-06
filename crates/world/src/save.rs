//! A world as JSON, and back. Tiles are rows of glyphs — the same characters
//! the terminal display draws — so a snapshot is readable by eye.

use std::collections::VecDeque;

use gemini::{arr, obj, Value};

use crate::{
    Command, Event, Form, Item, Npc, NpcId, Place, Player, PlayerId, Speech, Task, Tile, Want,
    World, H, W,
};

fn want_json(w: &Want) -> Value {
    obj! {
        "item" => w.item.as_str(), "amount" => w.amount, "given" => w.given,
        "reward" => list(&w.reward), "repeat" => w.repeat, "words" => w.words.as_str(),
    }
}

fn unwant(v: &Value) -> Result<Option<Want>, String> {
    if v.is_null() {
        return Ok(None);
    }
    Ok(Some(Want {
        item: v.get("item").to_text(),
        amount: v.get("amount").as_u32().unwrap_or(1),
        given: v.get("given").as_u32().unwrap_or(0),
        reward: unlist(v.get("reward"))?,
        repeat: v.get("repeat").as_bool().unwrap_or(false),
        words: v.get("words").to_text(),
    }))
}

const EVENTS_KEPT: usize = 40;

fn list(v: &[(String, u32)]) -> Value {
    Value::Arr(v.iter().map(|(k, n)| arr![k.as_str(), *n]).collect())
}

fn unlist(v: &Value) -> Result<Vec<(String, u32)>, String> {
    v.as_arr()
        .iter()
        .map(|p| Ok((p.at(0).to_text(), p.at(1).as_u32().ok_or("bad count")?)))
        .collect()
}

fn cmds(v: &[Command]) -> Value {
    Value::Arr(v.iter().map(Command::to_json).collect())
}

fn uncmds(v: &Value) -> Result<Vec<Command>, String> {
    v.as_arr().iter().map(Command::from_json).collect()
}

impl Task {
    fn to_json(&self) -> Value {
        match self {
            Task::Idle => obj! {"t" => "idle"},
            Task::Walk { to, then } => obj! {"t" => "walk", "to" => arr![to.0, to.1]}
                .with_opt("then", then.as_ref().map(|c| c.to_json())),
            Task::Gather {
                resource,
                want,
                got,
            } => obj! {"t" => "gather", "resource" => resource.as_str(), "got" => *got}
                .with_opt("want", *want),
            Task::Build { site } => obj! {"t" => "build", "site" => site.as_str()},
        }
    }
    fn from_json(v: &Value) -> Result<Task, String> {
        Ok(match v.get("t").as_str() {
            Some("idle") | None => Task::Idle,
            Some("walk") => Task::Walk {
                to: (
                    v.get("to").at(0).as_i64().ok_or("bad walk")? as i32,
                    v.get("to").at(1).as_i64().ok_or("bad walk")? as i32,
                ),
                then: match v.get("then") {
                    Value::Null => None,
                    c => Some(Box::new(Command::from_json(c)?)),
                },
            },
            Some("gather") => Task::Gather {
                resource: v.get("resource").to_text(),
                want: v.get("want").as_u32(),
                got: v.get("got").as_u32().unwrap_or(0),
            },
            Some("build") => Task::Build {
                site: v.get("site").to_text(),
            },
            other => return Err(format!("unknown task {other:?}")),
        })
    }
}

impl World {
    /// What a display needs and nothing else: the tiles as glyph rows, and
    /// every place, NPC and character with a position and what they are doing.
    /// The renderer never sees text meant for the model.
    pub fn scene(&self, me: Option<PlayerId>) -> Value {
        let rows: Vec<String> = (0..H)
            .map(|y| (0..W).map(|x| self.tile(x, y).glyph()).collect())
            .collect();
        let places: Vec<Value> = self
            .places
            .iter()
            .map(|p| {
                let (w, h) = p.size();
                let total = p.form.work().max(1);
                let progress = if !p.needs.is_empty() {
                    0.0
                } else {
                    (p.work.min(total) as f64) / (total as f64)
                };
                obj! {
                    "name" => p.name.as_str(), "x" => p.x, "y" => p.y,
                    "form" => p.form.name(), "w" => w, "h" => h,
                    "built" => p.built(), "progress" => progress,
                }
                .with_opt("resource", p.resource.as_deref())
                .with_opt("style", p.style.as_deref())
            })
            .collect();
        let npcs: Vec<Value> = self
            .npcs
            .iter()
            .map(|n| {
                let doing = if matches!(n.task, Task::Walk { .. }) { "walk" } else { "idle" };
                obj! {"name" => n.name.as_str(), "x" => n.x, "y" => n.y, "holds" => list(&n.holds), "doing" => doing}
                    .with_opt("wants", n.want.as_ref().map(Want::text))
            })
            .collect();
        let players: Vec<Value> = self
            .players
            .iter()
            .map(|p| {
                let (doing, res) = match &p.task {
                    Task::Idle => ("idle", None),
                    Task::Walk { .. } => ("walk", None),
                    Task::Gather { resource, .. } => ("gather", Some(resource.as_str())),
                    Task::Build { .. } => ("build", None),
                };
                obj! {"name" => p.name.as_str(), "x" => p.x, "y" => p.y, "doing" => doing, "me" => Some(p.id) == me, "carrying" => list(&p.inventory)}
                    .with_opt("resource", res)
            })
            .collect();
        // What was said lately, for speech bubbles.
        let speech: Vec<Value> = self
            .events
            .iter()
            .rev()
            .filter(|e| (e.kind == "say" || e.kind == "voice") && e.tick + 8 >= self.tick)
            .take(12)
            .map(|e| {
                let said = e
                    .text
                    .strip_prefix("says \"")
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(&e.text);
                obj! {"name" => e.name.as_str(), "text" => said, "tick" => e.tick}
            })
            .collect();
        obj! {
            "w" => W, "h" => H, "tick" => self.tick,
            "tiles" => rows, "places" => places, "npcs" => npcs, "players" => players,
            "speech" => speech,
        }
    }

    /// One NPC's standing, shaped like a player's, for the script that runs them.
    pub fn npc_status(&self, id: NpcId) -> Value {
        let Some(n) = self.npc(id) else {
            return Value::Null;
        };
        let doing = match &n.task {
            Task::Walk { to, .. } => format!("walking to {}", self.label(*to)),
            _ => "idle".to_string(),
        };
        obj! {
            "name" => n.name.as_str(), "x" => n.x, "y" => n.y,
            "place" => self.place_at(n.x, n.y).map(|pl| pl.name.clone()),
            "doing" => doing, "carrying" => list(&n.holds),
            "bank" => Value::Arr(Vec::new()), "skills" => Value::Arr(Vec::new()), "recipes" => Value::Arr(Vec::new()),
            "home" => arr![n.home.0, n.home.1],
        }
        .with_opt("wants", n.want.as_ref().map(Want::text))
    }

    /// One character's standing, as fields a header can lay out.
    pub fn status(&self, me: PlayerId) -> Value {
        let Some(p) = self.player(me) else {
            return Value::Null;
        };
        let doing = match &p.task {
            Task::Idle => "idle".to_string(),
            Task::Walk { to, then: None } => format!("walking to {}", self.label(*to)),
            Task::Walk {
                to,
                then: Some(next),
            } => format!("walking to {} to {next}", self.label(*to)),
            Task::Gather {
                resource,
                want: Some(w),
                got,
            } => format!("gathering {resource} ({got}/{w})"),
            Task::Gather {
                resource,
                want: None,
                got,
            } => format!("gathering {resource} ({got} so far)"),
            Task::Build { site } => format!("building {site}"),
        };
        let mut then: Vec<String> = p.queue.iter().map(|c| c.to_string()).collect();
        if let Some((rname, _)) = &p.looping {
            then.push(format!("repeat '{rname}'"));
        }
        let skills: Vec<Value> =
            p.xp.iter()
                .map(|(sk, _)| arr![sk.as_str(), p.level(sk)])
                .collect();
        let recipes: Vec<Value> = p
            .recipes
            .iter()
            .map(|(n, steps)| {
                arr![
                    n.as_str(),
                    steps
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ]
            })
            .collect();
        obj! {
            "name" => p.name.as_str(), "x" => p.x, "y" => p.y,
            "place" => self.place_at(p.x, p.y).map(|pl| pl.name.clone()),
            "doing" => doing, "then" => then.join(", "),
            "carrying" => list(&p.inventory), "bank" => list(&p.bank),
            "skills" => skills, "recipes" => recipes, "script" => p.script.is_some(),
            "offer" => p.want.as_ref().map(Want::text),
        }
    }

    pub fn to_json(&self) -> Value {
        let rows: Vec<String> = (0..H)
            .map(|y| (0..W).map(|x| self.tile(x, y).glyph()).collect())
            .collect();
        let places: Vec<Value> = self
            .places
            .iter()
            .map(|p| {
                obj! {
                    "name" => p.name.as_str(), "x" => p.x, "y" => p.y, "description" => p.description.as_str(),
                    "form" => p.form.name(), "needs" => list(&p.needs), "work" => p.work,
                }
                .with_opt("resource", p.resource.as_deref())
                .with_opt("skill", p.skill.as_deref())
                .with_opt("founder", p.founder.map(|f| f.0))
                .with_opt("style", p.style.as_deref())
            })
            .collect();
        let npcs: Vec<Value> = self
            .npcs
            .iter()
            .map(|n| {
                obj! {
                    "id" => n.id.0, "name" => n.name.as_str(), "persona" => n.persona.as_str(), "x" => n.x, "y" => n.y,
                    "creator" => n.creator.0, "holds" => list(&n.holds), "home" => arr![n.home.0, n.home.1],
                    "task" => n.task.to_json(), "memory" => n.memory.clone(), "script_tick" => n.script_tick,
                }
                .with_opt("want", n.want.as_ref().map(want_json))
                .with_opt("script", n.script.as_deref())
            })
            .collect();
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|i| obj! {"name" => i.name.as_str(), "description" => i.description.as_str(), "recipe" => list(&i.recipe), "maker" => i.maker.0})
            .collect();
        let players: Vec<Value> = self
            .players
            .iter()
            .map(|p| {
                obj! {
                    "id" => p.id.0, "name" => p.name.as_str(), "x" => p.x, "y" => p.y,
                    "inventory" => list(&p.inventory), "bank" => list(&p.bank), "xp" => list(&p.xp),
                    "task" => p.task.to_json(),
                    "queue" => cmds(p.queue.iter().cloned().collect::<Vec<_>>().as_slice()),
                    "last_plan" => cmds(&p.last_plan),
                    "memory" => p.memory.clone(), "script_tick" => p.script_tick,
                    "recipes" => p.recipes.iter().map(|(n, steps)| arr![n.as_str(), cmds(steps)]).collect::<Vec<_>>(),
                }
                .with_opt("looping", p.looping.as_ref().map(|(n, steps)| arr![n.as_str(), cmds(steps)]))
                .with_opt("script", p.script.as_deref())
                .with_opt("want", p.want.as_ref().map(want_json))
            })
            .collect();
        let skip = self.events.len().saturating_sub(EVENTS_KEPT);
        let events: Vec<Value> = self
            .events
            .iter()
            .skip(skip)
            .map(|e| obj! {"tick" => e.tick, "name" => e.name.as_str(), "text" => e.text.as_str(), "kind" => e.kind})
            .collect();
        let speeches: Vec<Value> = self
            .speeches
            .iter()
            .map(|s| obj! {"tick" => s.tick, "speaker" => s.speaker.0, "listener" => s.listener.0, "text" => s.text.as_str()})
            .collect();
        obj! {
            "seed" => self.seed,
            "tick" => self.tick,
            "next_id" => self.next_id,
            "next_npc" => self.next_npc,
            "tiles" => rows,
            "places" => places,
            "npcs" => npcs,
            "players" => players,
            "events" => events,
            "items" => items,
            "speeches" => speeches,
        }
    }

    pub fn from_json(v: &Value) -> Result<World, String> {
        let rows = v.get("tiles").as_arr();
        if rows.len() != H as usize {
            return Err("wrong map height".into());
        }
        let mut tiles = Vec::with_capacity((W * H) as usize);
        for row in rows {
            let row = row.as_str().ok_or("bad map row")?;
            if row.chars().count() != W as usize {
                return Err("wrong map width".into());
            }
            for c in row.chars() {
                tiles.push(match c {
                    '.' => Tile::Grass,
                    '~' => Tile::Water,
                    'T' => Tile::Forest,
                    '^' => Tile::Hill,
                    '=' => Tile::Road,
                    '#' => Tile::Town,
                    other => return Err(format!("unknown tile {other:?}")),
                });
            }
        }
        let i32_of = |v: &Value, k: &str| {
            v.get(k)
                .as_i64()
                .map(|n| n as i32)
                .ok_or(format!("missing {k}"))
        };
        let places = v
            .get("places")
            .as_arr()
            .iter()
            .map(|p| {
                Ok(Place {
                    name: p.get("name").to_text(),
                    x: i32_of(p, "x")?,
                    y: i32_of(p, "y")?,
                    resource: p.get("resource").as_str().map(str::to_string),
                    skill: p.get("skill").as_str().map(str::to_string),
                    description: p.get("description").to_text(),
                    founder: p.get("founder").as_u32().map(PlayerId),
                    form: Form::parse(p.get("form").as_str().unwrap_or("")).unwrap_or(Form::Banner),
                    style: p.get("style").as_str().map(str::to_string),
                    needs: unlist(p.get("needs"))?,
                    work: p.get("work").as_u32().unwrap_or(0),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let npcs = v
            .get("npcs")
            .as_arr()
            .iter()
            .map(|n| {
                Ok(Npc {
                    id: NpcId(n.get("id").as_u32().ok_or("npc without id")?),
                    name: n.get("name").to_text(),
                    persona: n.get("persona").to_text(),
                    x: i32_of(n, "x")?,
                    y: i32_of(n, "y")?,
                    creator: PlayerId(n.get("creator").as_u32().unwrap_or(0)),
                    holds: unlist(n.get("holds"))?,
                    want: unwant(n.get("want"))?,
                    home: match n.get("home") {
                        Value::Null => (i32_of(n, "x")?, i32_of(n, "y")?),
                        h => (
                            h.at(0).as_i64().unwrap_or(0) as i32,
                            h.at(1).as_i64().unwrap_or(0) as i32,
                        ),
                    },
                    task: Task::from_json(n.get("task"))?,
                    script: n.get("script").as_str().map(str::to_string),
                    memory: n.get("memory").clone(),
                    script_tick: n
                        .get("script_tick")
                        .as_f64()
                        .map(|t| t as u64)
                        .unwrap_or(u64::MAX),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let players = v
            .get("players")
            .as_arr()
            .iter()
            .map(|p| {
                Ok(Player {
                    id: PlayerId(p.get("id").as_u32().ok_or("player without id")?),
                    name: p.get("name").to_text(),
                    x: i32_of(p, "x")?,
                    y: i32_of(p, "y")?,
                    inventory: unlist(p.get("inventory"))?,
                    bank: unlist(p.get("bank"))?,
                    xp: unlist(p.get("xp"))?,
                    task: Task::from_json(p.get("task"))?,
                    queue: VecDeque::from(uncmds(p.get("queue"))?),
                    last_plan: uncmds(p.get("last_plan"))?,
                    recipes: p
                        .get("recipes")
                        .as_arr()
                        .iter()
                        .map(|r| Ok((r.at(0).to_text(), uncmds(r.at(1))?)))
                        .collect::<Result<Vec<_>, String>>()?,
                    script: p.get("script").as_str().map(str::to_string),
                    memory: p.get("memory").clone(),
                    script_tick: p
                        .get("script_tick")
                        .as_f64()
                        .map(|f| f as u64)
                        .unwrap_or(u64::MAX),
                    want: unwant(p.get("want"))?,
                    looping: match p.get("looping") {
                        Value::Null => None,
                        l => Some((l.at(0).to_text(), uncmds(l.at(1))?)),
                    },
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let events = v
            .get("events")
            .as_arr()
            .iter()
            .map(|e| Event {
                tick: e.get("tick").as_f64().unwrap_or(0.0) as u64,
                name: e.get("name").to_text(),
                text: e.get("text").to_text(),
                kind: match e.get("kind").as_str() {
                    Some("say") => "say",
                    Some("voice") => "voice",
                    Some("join") => "join",
                    Some("script") => "script",
                    Some("build") => "build",
                    Some("give") => "give",
                    Some("craft") => "craft",
                    _ => "note",
                },
            })
            .collect();
        let speeches = v
            .get("speeches")
            .as_arr()
            .iter()
            .map(|s| {
                Ok(Speech {
                    tick: s.get("tick").as_f64().unwrap_or(0.0) as u64,
                    speaker: PlayerId(s.get("speaker").as_u32().ok_or("speech without speaker")?),
                    listener: NpcId(
                        s.get("listener")
                            .as_u32()
                            .ok_or("speech without listener")?,
                    ),
                    text: s.get("text").to_text(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let items = v
            .get("items")
            .as_arr()
            .iter()
            .map(|i| {
                Ok(Item {
                    name: i.get("name").to_text(),
                    description: i.get("description").to_text(),
                    recipe: unlist(i.get("recipe"))?,
                    maker: PlayerId(i.get("maker").as_u32().unwrap_or(0)),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(World {
            seed: v.get("seed").as_f64().unwrap_or(0.0) as u64,
            tick: v.get("tick").as_f64().unwrap_or(0.0) as u64,
            tiles,
            places,
            npcs,
            players,
            events,
            items,
            speeches,
            next_id: v.get("next_id").as_u32().unwrap_or(1),
            next_npc: v.get("next_npc").as_u32().unwrap_or(1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_world_survives_the_trip() {
        let mut w = World::new(11);
        let me = w.join("Ada");
        w.apply(
            me,
            &Command::CreateNpc {
                name: "Wren".into(),
                persona: "Talks to birds.".into(),
            },
        )
        .unwrap();
        w.apply(
            me,
            &Command::Say {
                text: "hello".into(),
            },
        )
        .unwrap();
        w.plan(
            me,
            vec![
                Command::Gather {
                    resource: "wood".into(),
                    amount: Some(4),
                },
                Command::Bank,
            ],
        )
        .unwrap();
        w.apply(
            me,
            &Command::SaveRecipe {
                name: "woodrun".into(),
            },
        )
        .unwrap();
        w.apply(
            me,
            &Command::RunRecipe {
                name: "woodrun".into(),
                forever: true,
            },
        )
        .unwrap();
        for _ in 0..25 {
            w.step();
        }
        let text = w.to_json().to_string();
        let back = World::from_json(&Value::parse(&text).unwrap()).unwrap();
        assert_eq!(w, back);
        assert_eq!(back.speeches().len(), 1);
        assert!(text.contains("\"tiles\":[\""));
        // Events are trimmed on save, and only events.
        for _ in 0..300 {
            w.step();
        }
        let back = World::from_json(&w.to_json()).unwrap();
        assert!(back.events.len() <= EVENTS_KEPT);
        assert_eq!(back.players, w.players);
        assert!(World::from_json(&Value::parse("{}").unwrap()).is_err());
    }
}
