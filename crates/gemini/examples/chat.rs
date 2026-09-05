//! A streamed one-shot, and the model list.
//!
//! ```text
//! cargo run -p gemini --features native --example chat -- "why is the sky blue, in one line"
//! cargo run -p gemini --features native --example chat -- --models
//! GEMINI_MODEL=gemini-3.5-flash cargo run -p gemini --features native --example chat -- --think "17*23?"
//! ```

use std::io::Write;

use gemini::native::Client;
use gemini::{Delta, Level, Request, Thinking};

fn main() {
    let client = match Client::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--models") {
        match client.models() {
            Ok(models) => models.iter().for_each(|m| println!("{m}")),
            Err(e) => eprintln!("{e}"),
        }
        return;
    }
    let think = args.iter().any(|a| a == "--think");
    let prompt: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();
    let prompt = if prompt.is_empty() {
        "Say hello in five words.".to_string()
    } else {
        prompt.join(" ")
    };
    let model =
        std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.5-flash-lite".to_string());

    let mut req = Request::new(&model).user(prompt);
    if think {
        req = req
            .thinking(Thinking::Level(Level::Low))
            .include_thoughts(true);
    }
    let out = std::io::stdout();
    let result = client.stream(&req, |d| {
        let mut o = out.lock();
        match d {
            Delta::Text(t) => write!(o, "{t}").unwrap(),
            Delta::Thought(t) => write!(o, "\x1b[2m{t}\x1b[0m").unwrap(),
            Delta::Call { name, args, .. } => write!(o, "[call {name}({args})]").unwrap(),
        }
        o.flush().unwrap();
    });
    println!();
    match result {
        Ok(r) => eprintln!(
            "-- {} · finish {:?} · {} prompt + {} output (+{} thought) tokens · {} part(s)",
            r.model_version.as_deref().unwrap_or(&model),
            r.finish,
            r.usage.prompt,
            r.usage.output,
            r.usage.thoughts,
            r.content.parts.len()
        ),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
