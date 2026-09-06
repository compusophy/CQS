//! Lua for cqs, on piccolo: a pure-Rust, sandboxed, fuel-metered Lua VM.
//!
//! A script is a player's standing agent. A host runs it whenever the
//! character is idle: the script sees the same facts the pilot sees — `me`,
//! `places`, `people`, `tick` — plus a `memory` table that persists between
//! runs, and it acts through the same commands a keyboard would: `walk`,
//! `gather`, `bank`, `say`, `found`, `npc`. It cannot see or touch anything
//! else; it gets a fixed budget of instructions per run; and the world, not
//! the script, decides whether each step is legal. The script's output is
//! recorded in the ledger, so a replay never runs Lua at all.

use std::cell::RefCell;
use std::rc::Rc;

use gemini::{obj, Value};
use piccolo::table::NextValue;
use piccolo::{
    Callback, CallbackReturn, Closure, Context, Error as LuaError, Executor, Fuel, Lua, Stack,
    Table, Value as Lv,
};
use world::{Command, Form};

/// Instructions one run may spend.
pub const FUEL: i32 = 200_000;
/// Steps one run may issue.
pub const MAX_COMMANDS: usize = 8;
/// Lines one run may log.
pub const MAX_LOG: usize = 6;
const MEMORY_MAX_ENTRIES: usize = 64;
const MEMORY_MAX_DEPTH: usize = 4;
const STRING_MAX: usize = 400;

/// What one run produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub cmds: Vec<Command>,
    pub memory: Value,
    pub log: Vec<String>,
    pub error: Option<String>,
}

type Shared<T> = Rc<RefCell<T>>;

/// Run a script once against a character's status (`World::status`) and the
/// scene (`World::scene`) with its remembered `memory`.
pub fn run(source: &str, status: &Value, scene: &Value, memory: &Value) -> Outcome {
    let cmds: Shared<Vec<Command>> = Rc::new(RefCell::new(Vec::new()));
    let log: Shared<Vec<String>> = Rc::new(RefCell::new(Vec::new()));
    let me_x = status.get("x").as_i64().unwrap_or(0) as i32;
    let me_y = status.get("y").as_i64().unwrap_or(0) as i32;
    let me_name = status.get("name").to_text();
    let people = people_json(scene, &me_name, me_x, me_y);
    let places = places_json(scene, me_x, me_y);
    let distances: Rc<Vec<(String, i64)>> = Rc::new(
        people
            .as_arr()
            .iter()
            .chain(places.as_arr().iter())
            .map(|p| {
                (
                    p.get("name").to_text().to_ascii_lowercase(),
                    p.get("distance").as_i64().unwrap_or(99),
                )
            })
            .collect(),
    );

    let mut lua = Lua::core();
    let bind = {
        let cmds = cmds.clone();
        let log = log.clone();
        let distances = distances.clone();
        let me = me_table(status);
        let memory = memory.clone();
        let tick = scene.get("tick").as_i64().unwrap_or(0);
        lua.try_enter(move |ctx| {
            let _ = ctx.set_global("me", json_to_lua(ctx, &me));
            let _ = ctx.set_global("places", json_to_lua(ctx, &places));
            let _ = ctx.set_global("people", json_to_lua(ctx, &people));
            let _ = ctx.set_global("tick", Lv::Integer(tick));
            let _ = ctx.set_global(
                "memory",
                match memory {
                    Value::Obj(_) => json_to_lua(ctx, &memory),
                    _ => Lv::Table(Table::new(&ctx)),
                },
            );
            command(ctx, "walk", cmds.clone(), |st| {
                Ok(Command::MoveTo {
                    target: arg_str(st, 0).ok_or("walk needs a place or person")?,
                })
            });
            command(ctx, "gather", cmds.clone(), |st| {
                Ok(Command::Gather {
                    resource: arg_str(st, 0).ok_or("gather needs a resource")?,
                    amount: arg_int(st, 1).filter(|n| *n > 0).map(|n| n as u32),
                })
            });
            command(ctx, "bank", cmds.clone(), |_| Ok(Command::Bank));
            command(ctx, "stop", cmds.clone(), |_| Ok(Command::Stop));
            command(ctx, "say", cmds.clone(), |st| {
                Ok(Command::Say {
                    text: arg_str(st, 0).ok_or("say needs words")?,
                })
            });
            command(ctx, "found", cmds.clone(), |st| {
                Ok(Command::FoundPlace {
                    name: arg_str(st, 0).ok_or("found needs a name")?,
                    description: arg_str(st, 1).unwrap_or_default(),
                    resource: arg_str(st, 2),
                    skill: arg_str(st, 3),
                    form: Form::parse(&arg_str(st, 4).unwrap_or_default()).unwrap_or(Form::Banner),
                    style: arg_str(st, 5),
                })
            });
            command(ctx, "build", cmds.clone(), |st| {
                Ok(Command::Build {
                    site: arg_str(st, 0).ok_or("build needs a site")?,
                })
            });
            command(ctx, "abandon", cmds.clone(), |st| {
                Ok(Command::Abandon {
                    site: arg_str(st, 0).ok_or("abandon needs a site")?,
                })
            });
            command(ctx, "give", cmds.clone(), |st| {
                Ok(Command::Give {
                    item: arg_str(st, 0).ok_or("give needs an item")?,
                    amount: arg_int(st, 1).filter(|n| *n > 0).map(|n| n as u32),
                    to: arg_str(st, 2).ok_or("give needs someone to give to")?,
                })
            });
            command(ctx, "craft", cmds.clone(), |st| {
                Ok(Command::Craft {
                    item: arg_str(st, 0).ok_or("craft needs a name")?,
                    description: arg_str(st, 1).unwrap_or_default(),
                    from: world::goods(&arg_str(st, 2).unwrap_or_default()),
                })
            });
            command(ctx, "want", cmds.clone(), |st| {
                Ok(Command::SetWant {
                    npc: arg_str(st, 0).ok_or("want needs an npc")?,
                    item: arg_str(st, 1).ok_or("want needs an item")?,
                    amount: arg_int(st, 2).filter(|n| *n > 0).unwrap_or(1) as u32,
                    reward: world::goods(&arg_str(st, 3).unwrap_or_default()),
                    repeat: arg_bool(st, 4),
                    words: arg_str(st, 5).unwrap_or_default(),
                })
            });
            command(ctx, "npc", cmds.clone(), |st| {
                Ok(Command::CreateNpc {
                    name: arg_str(st, 0).ok_or("npc needs a name")?,
                    persona: arg_str(st, 1).unwrap_or_default(),
                })
            });
            {
                let distances = distances.clone();
                let near = Callback::from_fn(&ctx, move |_, _, mut stack| {
                    let who = arg_str(&stack, 0).unwrap_or_default().to_ascii_lowercase();
                    let d = distances.iter().find(|(n, _)| *n == who).map(|(_, d)| *d);
                    stack.clear();
                    stack.push_back(Lv::Boolean(d.map_or(false, |d| d <= 2)));
                    Ok(CallbackReturn::Return)
                });
                let _ = ctx.set_global("near", near);
            }
            {
                let distances = distances.clone();
                let dist = Callback::from_fn(&ctx, move |_, _, mut stack| {
                    let who = arg_str(&stack, 0).unwrap_or_default().to_ascii_lowercase();
                    let d = distances.iter().find(|(n, _)| *n == who).map(|(_, d)| *d);
                    stack.clear();
                    stack.push_back(d.map_or(Lv::Nil, Lv::Integer));
                    Ok(CallbackReturn::Return)
                });
                let _ = ctx.set_global("dist", dist);
            }
            {
                let log = log.clone();
                let cb = Callback::from_fn(&ctx, move |_, _, mut stack| {
                    let line: String = arg_str(&stack, 0)
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect();
                    let mut l = log.borrow_mut();
                    if l.len() < MAX_LOG {
                        l.push(line);
                    }
                    stack.clear();
                    Ok(CallbackReturn::Return)
                });
                let _ = ctx.set_global("log", cb);
            }
            Ok(())
        })
    };
    if let Err(e) = bind {
        return Outcome {
            error: Some(e.to_string()),
            memory: memory.clone(),
            ..Default::default()
        };
    }

    let executor = match lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    }) {
        Ok(ex) => ex,
        Err(e) => {
            return Outcome {
                error: Some(format!("does not compile: {e}")),
                memory: memory.clone(),
                ..Default::default()
            }
        }
    };

    // Run under fuel, in slices so the collector gets to work.
    let mut budget = FUEL;
    let mut finished = false;
    while budget > 0 && !finished {
        let slice = budget.min(16_384);
        let (done, used) = lua.enter(|ctx| {
            let ex = ctx.fetch(&executor);
            let mut fuel = Fuel::with(slice);
            let done = ex.step(ctx, &mut fuel);
            (done, slice - fuel.remaining())
        });
        budget -= used.max(1);
        finished = done;
    }
    let mut error = if finished {
        lua.enter(|ctx| match ctx.fetch(&executor).take_result::<()>(ctx) {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(_) => Some("the script did not finish".to_string()),
        })
    } else {
        Some(format!("out of fuel after {FUEL} instructions"))
    };
    let remembered = lua.enter(|ctx| lua_to_json(ctx.get_global("memory"), 0));
    let cmds = std::mem::take(&mut *cmds.borrow_mut());
    let log = std::mem::take(&mut *log.borrow_mut());
    if let Some(e) = &mut error {
        *e = e.chars().take(300).collect();
    }
    Outcome {
        cmds,
        memory: remembered,
        log,
        error,
    }
}

/// Register a global that turns its arguments into one `Command`.
fn command<'gc, F>(ctx: Context<'gc>, name: &'static str, cmds: Shared<Vec<Command>>, make: F)
where
    F: 'static + Fn(&Stack<'gc, '_>) -> Result<Command, &'static str>,
{
    let cb = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let cmd = match make(&stack) {
            Ok(c) => c,
            Err(why) => return Err(LuaError::from(Lv::String(ctx.intern(why.as_bytes())))),
        };
        {
            let mut list = cmds.borrow_mut();
            if list.len() >= MAX_COMMANDS {
                let msg = format!("a script may issue at most {MAX_COMMANDS} steps per run");
                return Err(LuaError::from(Lv::String(ctx.intern(msg.as_bytes()))));
            }
            list.push(cmd);
        }
        stack.clear();
        Ok(CallbackReturn::Return)
    });
    let _ = ctx.set_global(name, cb);
}

fn arg_str(stack: &Stack<'_, '_>, i: usize) -> Option<String> {
    match stack.get(i) {
        Lv::String(s) => Some(s.to_str_lossy().chars().take(STRING_MAX).collect()),
        Lv::Integer(n) => Some(n.to_string()),
        Lv::Number(f) => Some(f.to_string()),
        _ => None,
    }
}

fn arg_int(stack: &Stack<'_, '_>, i: usize) -> Option<i64> {
    match stack.get(i) {
        Lv::Integer(n) => Some(n),
        Lv::Number(f) => Some(f as i64),
        Lv::String(s) => s.to_str().ok()?.trim().parse().ok(),
        _ => None,
    }
}

fn arg_bool(stack: &Stack<'_, '_>, i: usize) -> bool {
    match stack.get(i) {
        Lv::Boolean(b) => b,
        Lv::Integer(n) => n != 0,
        Lv::String(s) => matches!(s.to_str().ok().map(str::trim), Some("true" | "yes" | "1")),
        _ => false,
    }
}

/// The `me` table: the status object with its pair-lists turned into maps.
fn me_table(status: &Value) -> Value {
    let map = |key: &str| -> Value {
        let mut m = Value::obj();
        for p in status.get(key).as_arr() {
            m.set(&p.at(0).to_text(), p.at(1).clone());
        }
        m
    };
    obj! {
        "name" => status.get("name").clone(),
        "x" => status.get("x").clone(),
        "y" => status.get("y").clone(),
        "place" => status.get("place").clone(),
        "doing" => status.get("doing").clone(),
        "carrying" => map("carrying"),
        "bank" => map("bank"),
        "skills" => map("skills"),
        "recipes" => map("recipes"),
        "home" => status.get("home").clone(),
        "wants" => status.get("wants").clone(),
    }
}

fn chebyshev(ax: i32, ay: i32, bx: i32, by: i32) -> i64 {
    (ax - bx).abs().max((ay - by).abs()) as i64
}

fn places_json(scene: &Value, mx: i32, my: i32) -> Value {
    Value::Arr(
        scene
            .get("places")
            .as_arr()
            .iter()
            .map(|p| {
                let (x, y) = (
                    p.get("x").as_i64().unwrap_or(0) as i32,
                    p.get("y").as_i64().unwrap_or(0) as i32,
                );
                obj! {
                    "name" => p.get("name").clone(), "x" => x, "y" => y,
                    "resource" => p.get("resource").clone(),
                    "form" => p.get("form").clone(), "built" => p.get("built").clone(),
                    "distance" => chebyshev(mx, my, x, y),
                }
            })
            .collect(),
    )
}

fn people_json(scene: &Value, me: &str, mx: i32, my: i32) -> Value {
    let mut out = Vec::new();
    for p in scene.get("players").as_arr() {
        if p.get("name").as_str() == Some(me) {
            continue;
        }
        let (x, y) = (
            p.get("x").as_i64().unwrap_or(0) as i32,
            p.get("y").as_i64().unwrap_or(0) as i32,
        );
        out.push(obj! {"name" => p.get("name").clone(), "x" => x, "y" => y, "npc" => false, "doing" => p.get("doing").clone(), "distance" => chebyshev(mx, my, x, y)});
    }
    for n in scene.get("npcs").as_arr() {
        let (x, y) = (
            n.get("x").as_i64().unwrap_or(0) as i32,
            n.get("y").as_i64().unwrap_or(0) as i32,
        );
        if n.get("name").as_str() == Some(me) {
            continue;
        }
        let doing = n.get("doing").as_str().unwrap_or("idle");
        out.push(obj! {"name" => n.get("name").clone(), "x" => x, "y" => y, "npc" => true, "doing" => doing, "wants" => n.get("wants").clone(), "distance" => chebyshev(mx, my, x, y)});
    }
    Value::Arr(out)
}

fn json_to_lua<'gc>(ctx: Context<'gc>, v: &Value) -> Lv<'gc> {
    match v {
        Value::Null => Lv::Nil,
        Value::Bool(b) => Lv::Boolean(*b),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                Lv::Integer(*n as i64)
            } else {
                Lv::Number(*n)
            }
        }
        Value::Str(s) => Lv::String(ctx.intern(s.as_bytes())),
        Value::Arr(a) => {
            let t = Table::new(&ctx);
            for (i, item) in a.iter().enumerate() {
                let _ = t.set(ctx, Lv::Integer(i as i64 + 1), json_to_lua(ctx, item));
            }
            Lv::Table(t)
        }
        Value::Obj(o) => {
            let t = Table::new(&ctx);
            for (k, item) in o {
                let _ = t.set(
                    ctx,
                    Lv::String(ctx.intern(k.as_bytes())),
                    json_to_lua(ctx, item),
                );
            }
            Lv::Table(t)
        }
    }
}

/// A Lua value back to JSON, bounded in depth, size and string length, so a
/// script cannot bloat its memory. Functions and userdata vanish.
fn lua_to_json(v: Lv<'_>, depth: usize) -> Value {
    match v {
        Lv::Nil => Value::Null,
        Lv::Boolean(b) => Value::Bool(b),
        Lv::Integer(n) => Value::from(n),
        Lv::Number(f) => Value::from(f),
        Lv::String(s) => Value::Str(s.to_str_lossy().chars().take(STRING_MAX).collect()),
        Lv::Table(t) => {
            if depth >= MEMORY_MAX_DEPTH {
                return Value::Null;
            }
            let mut pairs: Vec<(Value, Value)> = Vec::new();
            let mut key = Lv::Nil;
            loop {
                match t.next(key) {
                    NextValue::Found { key: k, value } => {
                        if pairs.len() >= MEMORY_MAX_ENTRIES {
                            break;
                        }
                        let kj = match k {
                            Lv::Integer(n) => Value::from(n),
                            Lv::Number(f) => Value::from(f),
                            Lv::String(s) => {
                                Value::Str(s.to_str_lossy().chars().take(64).collect())
                            }
                            Lv::Boolean(b) => Value::Str(b.to_string()),
                            _ => Value::Null,
                        };
                        let vj = lua_to_json(value, depth + 1);
                        if !kj.is_null() && !matches!(value, Lv::Function(_) | Lv::UserData(_)) {
                            pairs.push((kj, vj));
                        }
                        key = k;
                    }
                    NextValue::Last | NextValue::NotFound => break,
                }
            }
            // A sequence 1..n stays an array; anything else is an object.
            let mut ints: Vec<(i64, &Value)> = pairs
                .iter()
                .filter_map(|(k, v)| k.as_i64().map(|i| (i, v)))
                .collect();
            if !pairs.is_empty() && ints.len() == pairs.len() {
                ints.sort_by_key(|(i, _)| *i);
                if ints
                    .iter()
                    .enumerate()
                    .all(|(i, (k, _))| *k == i as i64 + 1)
                {
                    return Value::Arr(ints.into_iter().map(|(_, v)| v.clone()).collect());
                }
            }
            Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_text(), v)).collect())
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world::World;

    fn fixture() -> (Value, Value) {
        let mut w = World::new(7);
        let me = w.join("Ada");
        w.apply(
            me,
            &Command::CreateNpc {
                name: "Wren".into(),
                persona: "A forager.".into(),
            },
        )
        .unwrap();
        (w.status(me), w.scene(Some(me)))
    }

    #[test]
    fn a_script_sees_the_world_and_issues_steps() {
        let (status, scene) = fixture();
        let src = r#"
            log("at " .. tostring(me.place) .. " with " .. tostring(me.bank.wood or 0) .. " wood")
            if near("Wren") then say("hello wren") end
            for _, p in ipairs(places) do
              if p.resource == "iron" then walk(p.name) end
            end
            gather("wood", 5)
            bank()
            memory.runs = (memory.runs or 0) + 1
            memory.seen = { "iron", "wood" }
        "#;
        let out = run(src, &status, &scene, &Value::Null);
        assert_eq!(out.error, None, "{:?}", out);
        assert_eq!(
            out.cmds,
            vec![
                Command::Say {
                    text: "hello wren".into()
                },
                Command::MoveTo {
                    target: "Iron Hill".into()
                },
                Command::Gather {
                    resource: "wood".into(),
                    amount: Some(5)
                },
                Command::Bank,
            ]
        );
        assert_eq!(out.log, vec!["at Town with 0 wood".to_string()]);
        assert_eq!(out.memory.get("runs").as_i64(), Some(1));
        assert_eq!(out.memory.get("seen").at(1).as_str(), Some("wood"));
        // Memory comes back next run.
        let again = run(
            "memory.runs = memory.runs + 1",
            &status,
            &scene,
            &out.memory,
        );
        assert_eq!(again.memory.get("runs").as_i64(), Some(2));
        assert!(again.cmds.is_empty());
    }

    #[test]
    fn scripts_are_bounded() {
        let (status, scene) = fixture();
        let spin = run("while true do end", &status, &scene, &Value::Null);
        assert!(
            spin.error.as_deref().unwrap_or("").contains("fuel"),
            "{:?}",
            spin.error
        );
        let greedy = run("for i = 1, 20 do bank() end", &status, &scene, &Value::Null);
        assert_eq!(greedy.cmds.len(), MAX_COMMANDS);
        assert!(
            greedy.error.as_deref().unwrap_or("").contains("at most"),
            "{:?}",
            greedy.error
        );
        let broken = run("this is not lua", &status, &scene, &Value::Null);
        assert!(
            broken.error.as_deref().unwrap_or("").contains("compile"),
            "{:?}",
            broken.error
        );
        let bad_call = run("gather()", &status, &scene, &Value::Null);
        assert!(
            bad_call.error.as_deref().unwrap_or("").contains("resource"),
            "{:?}",
            bad_call.error
        );
        // No io, no os: the sandbox is the core library only.
        let escape = run("io.write('x')", &status, &scene, &Value::Null);
        assert!(escape.error.is_some());
        let bloat = run(
            "for i = 1, 1000 do memory[i] = i end",
            &status,
            &scene,
            &Value::Null,
        );
        assert!(bloat.memory.as_arr().len() <= MEMORY_MAX_ENTRIES);
    }
}
