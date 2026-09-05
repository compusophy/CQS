//! project cqs at the terminal. The world ticks in real time on its own
//! thread; the prompt is yours whenever you want it.
//!
//! ```text
//! cqs                        play: a prompt per line, `/` for raw commands
//! cqs --script "go mine iron" "chop 10 wood then bank it"   run prompts, then exit
//! cqs --offline              keyword pilot, no model, no key
//! cqs --name Ann --seed 9 --tps 2
//! ```

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gemini::native::{dotenv, Client};
use world::{pilot, Command, World};

fn main() {
    dotenv();
    let mut args = std::env::args().skip(1);
    let mut name = "Kyle".to_string();
    let mut seed = 7u64;
    let mut tps: Option<f64> = None;
    let mut offline = false;
    let mut script: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => name = args.next().unwrap_or(name),
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(seed),
            "--tps" => tps = args.next().and_then(|s| s.parse().ok()),
            "--offline" => offline = true,
            "--script" => script.extend(args.by_ref()),
            "-h" | "--help" => {
                println!("cqs [--name N] [--seed S] [--tps T] [--offline] [--script PROMPT...]");
                return;
            }
            other => script.push(other.to_string()),
        }
    }
    let scripted = !script.is_empty();
    // Scripts run the clock fast so a demo shows a whole plan in seconds.
    let tps = tps
        .unwrap_or(if scripted { 8.0 } else { 1.0 })
        .clamp(0.1, 100.0);
    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| pilot::DEFAULT_MODEL.to_string());
    let client = if offline {
        None
    } else {
        match Client::from_env() {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("({e}; piloting offline with keywords)");
                None
            }
        }
    };

    let world = Arc::new(Mutex::new(World::new(seed)));
    let me;
    {
        let mut w = world.lock().unwrap();
        me = w.join(name.clone());
        // Somebody else is already at work, on a recipe, so the world reads as
        // inhabited and the loop system is visibly alive.
        let ann = w.join("Ann");
        w.plan(
            ann,
            vec![
                Command::Gather {
                    resource: "wood".into(),
                    amount: Some(6),
                },
                Command::Bank,
            ],
        )
        .unwrap();
        w.apply(
            ann,
            &Command::SaveRecipe {
                name: "woodrun".into(),
            },
        )
        .unwrap();
        w.apply(
            ann,
            &Command::RunRecipe {
                name: "woodrun".into(),
                forever: true,
            },
        )
        .unwrap();
        for _ in 0..20 {
            w.step();
        }
    }

    // The clock. The world advances whether or not anyone is typing.
    {
        let world = Arc::clone(&world);
        let period = Duration::from_secs_f64(1.0 / tps);
        thread::spawn(move || loop {
            thread::sleep(period);
            world.lock().unwrap().step();
        });
    }

    // The voices. Whenever someone speaks within earshot of an NPC — now, or
    // three steps into a plan — the NPC answers, off the clock's thread.
    if let Some(client) = client.clone() {
        let world = Arc::clone(&world);
        let model = model.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(250));
            let speeches = world.lock().unwrap().take_speeches();
            for s in speeches {
                let (npc, view, speaker) = {
                    let w = world.lock().unwrap();
                    let Some(npc) = w.npc(s.listener).cloned() else {
                        continue;
                    };
                    let speaker = w
                        .player(s.speaker)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    (npc, w.describe(s.speaker), speaker)
                };
                match client.generate(&pilot::voice(&model, &npc, &view, &speaker, &s.text)) {
                    Ok(resp) => {
                        let reply = resp.text();
                        if !reply.trim().is_empty() {
                            world.lock().unwrap().npc_says(npc.id, &reply);
                            println!("\n{} says \"{}\"", npc.name, reply.trim());
                        }
                    }
                    Err(e) => println!("\n({} is silent: {e})", npc.name),
                }
            }
        });
    }

    println!(
        "project cqs — seed {seed}, {tps} tick/s, pilot: {}",
        client
            .as_ref()
            .map(|_| model.as_str())
            .unwrap_or("offline keywords")
    );
    {
        let w = world.lock().unwrap();
        print!("{}", w.ascii());
        println!("{}", w.describe(me));
    }

    let show = |world: &Mutex<World>| {
        let w = world.lock().unwrap();
        print!("{}", w.ascii());
        println!("{}", w.describe(me));
    };

    let run = |line: &str| -> bool {
        let line = line.trim();
        if line.is_empty() {
            return true;
        }
        if let Some(raw) = line.strip_prefix('/') {
            match raw.split_whitespace().next().unwrap_or("") {
                "quit" | "q" | "exit" => return false,
                "map" | "look" => show(&world),
                _ => {
                    println!("commands: /look /map /quit — anything else is said to your character")
                }
            }
            return true;
        }
        // The view is taken now; the world keeps ticking while the model thinks.
        let view = world.lock().unwrap().describe(me);
        let t0 = Instant::now();
        let cmds: Vec<Command> = match &client {
            Some(client) => match client.generate(&pilot::request(&model, &view, line)) {
                Ok(resp) => {
                    let ms = t0.elapsed().as_millis();
                    let mut cmds = Vec::new();
                    for c in pilot::commands(&resp) {
                        match c {
                            Ok(c) => cmds.push(c),
                            Err(e) => println!("  pilot: dropped a bad call ({e})"),
                        }
                    }
                    if cmds.is_empty() {
                        let text = resp.text();
                        println!("  pilot: no call, said {text:?}  [{ms} ms]");
                        if text.trim().is_empty() {
                            pilot::guess(line)
                        } else {
                            vec![Command::Say { text }]
                        }
                    } else {
                        let shown: Vec<String> = cmds.iter().map(|c| c.to_string()).collect();
                        println!(
                            "  pilot: {}  [{ms} ms, {} tokens]",
                            shown.join(" → "),
                            resp.usage.total
                        );
                        cmds
                    }
                }
                Err(e) => {
                    println!("  pilot: {e}; falling back to keywords");
                    pilot::guess(line)
                }
            },
            None => {
                let cmds = pilot::guess(line);
                let shown: Vec<String> = cmds.iter().map(|c| c.to_string()).collect();
                println!("  pilot (offline): {}", shown.join(" → "));
                cmds
            }
        };
        let ack = world.lock().unwrap().plan(me, cmds);
        match ack {
            Ok(ack) => println!("{ack}"),
            Err(why) => println!("  x {why}"),
        }
        if scripted {
            // Let the fast clock show the plan unfolding, and a voice answer.
            thread::sleep(Duration::from_millis(2500));
        }
        show(&world);
        true
    };

    if scripted {
        for line in script {
            println!("> {line}");
            if !run(&line) {
                break;
            }
        }
        return;
    }
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if !run(&line) {
            break;
        }
    }
}
