//! Accounting: what one pilot call and one voice call cost in tokens, measured
//! against the live model with the same request builders the host uses.
//! `cargo run -p cqs --example usage`
use gemini::native::Client;
use gemini::{Request, Usage};
use world::{pilot, Command, World};

fn send(client: &Client, label: &str, req: &Request) -> Usage {
    let bytes = req.body().to_string().len();
    match client.generate(req) {
        Ok(r) => {
            let u = r.usage;
            println!(
                "{label:<28} body {bytes:>5} B | prompt {:>5} | output {:>4} | thoughts {:>4} | cached {:>4} | total {:>5}",
                u.prompt, u.output, u.thoughts, u.cached, u.total
            );
            u
        }
        Err(e) => {
            println!("{label:<28} error: {e}");
            Usage::default()
        }
    }
}

fn main() {
    let client = Client::from_env().expect("GEMINI_API_KEY");
    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| pilot::DEFAULT_MODEL.to_string());
    let mut w = World::new(7);
    let me = w.join("Ada");
    w.apply(
        me,
        &Command::CreateNpc {
            name: "Old Wren".into(),
            persona: "A retired fisher who mends nets by the well and knows every rumour in town."
                .into(),
        },
    )
    .unwrap();
    let view = w.describe(me);
    println!("model {model}; view is {} chars\n", view.len());
    let mut total = Usage::default();
    for (label, words) in [
        ("pilot: simple", "go chop some wood and bank it"),
        (
            "pilot: creation",
            "build a blacksmith's forge here with a gruff smith called Brannock",
        ),
        (
            "pilot: script",
            "whenever I have 20 wood in my pack, bank it, otherwise keep chopping",
        ),
    ] {
        let u = send(&client, label, &pilot::request(&model, &view, words));
        total.prompt += u.prompt;
        total.output += u.output;
        total.thoughts += u.thoughts;
        total.total += u.total;
    }
    let npc = w.npcs_near(me).into_iter().next().expect("npc").clone();
    let u = send(
        &client,
        "voice: npc answers",
        &pilot::voice(
            &model,
            &npc,
            &view,
            "Ada",
            "hello Wren, any news from the river?",
        ),
    );
    total.prompt += u.prompt;
    total.output += u.output;
    total.thoughts += u.thoughts;
    total.total += u.total;
    println!(
        "\nfour calls: prompt {} | output {} | thoughts {} | total {}",
        total.prompt, total.output, total.thoughts, total.total
    );
}
