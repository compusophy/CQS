//! A world as JSON, and back. Tiles are rows of glyphs — the same characters
//! the terminal display draws — so a snapshot is readable by eye.

use std::collections::VecDeque;

use gemini::{arr, obj, Value};

use crate::{Command, Event, Npc, NpcId, Place, Player, PlayerId, Speech, Task, Tile, World, H, W};

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
            other => return Err(format!("unknown task {other:?}")),
        })
    }
}

impl World {
    pub fn to_json(&self) -> Value {
        let rows: Vec<String> = (0..H)
            .map(|y| (0..W).map(|x| self.tile(x, y).glyph()).collect())
            .collect();
        let places: Vec<Value> = self
            .places
            .iter()
            .map(|p| {
                obj! {"name" => p.name.as_str(), "x" => p.x, "y" => p.y, "description" => p.description.as_str()}
                    .with_opt("resource", p.resource.as_deref())
                    .with_opt("skill", p.skill.as_deref())
                    .with_opt("founder", p.founder.map(|f| f.0))
            })
            .collect();
        let npcs: Vec<Value> = self
            .npcs
            .iter()
            .map(|n| obj! {"id" => n.id.0, "name" => n.name.as_str(), "persona" => n.persona.as_str(), "x" => n.x, "y" => n.y, "creator" => n.creator.0})
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
                    "recipes" => p.recipes.iter().map(|(n, steps)| arr![n.as_str(), cmds(steps)]).collect::<Vec<_>>(),
                }
                .with_opt("looping", p.looping.as_ref().map(|(n, steps)| arr![n.as_str(), cmds(steps)]))
            })
            .collect();
        let skip = self.events.len().saturating_sub(EVENTS_KEPT);
        let events: Vec<Value> = self
            .events
            .iter()
            .skip(skip)
            .map(|e| obj! {"tick" => e.tick, "name" => e.name.as_str(), "text" => e.text.as_str()})
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
        Ok(World {
            seed: v.get("seed").as_f64().unwrap_or(0.0) as u64,
            tick: v.get("tick").as_f64().unwrap_or(0.0) as u64,
            tiles,
            places,
            npcs,
            players,
            events,
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
        let me = w.join("Kyle");
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
