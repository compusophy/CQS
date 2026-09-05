//! The world: small, deterministic, real-time, and the only memory the game has.
//!
//! A player never talks to the model directly. They talk to their character;
//! the pilot (`pilot.rs`) asks the model which `Command`s the words mean; the
//! world applies them and keeps ticking on its own clock. Every command is
//! plain data, so the same `apply` runs on a server, in a browser, or in a
//! test — and a transcript of commands replays a world exactly. There is no
//! chat history anywhere: the world *is* the context, rendered fresh by
//! `describe` for every prompt.
//!
//! Players build it. A place can be founded anywhere with any resource a
//! player can name; an NPC can be brought into being with a persona; a plan of
//! steps can be saved as a recipe and run forever. The world only validates —
//! it never invents — so everything in it was said by someone.

pub mod ledger;
pub mod pilot;
pub mod save;

use std::collections::VecDeque;
use std::fmt;

pub const W: i32 = 32;
pub const H: i32 = 18;

/// Caps that keep one loud player from filling the world.
pub const MAX_NPCS_PER_PLAYER: usize = 5;
pub const MAX_PLACES_PER_PLAYER: usize = 8;
pub const MAX_RECIPES: usize = 12;
pub const MAX_QUEUE: usize = 24;
pub const NAME_MAX: usize = 24;
pub const WORD_MAX: usize = 20;
pub const TEXT_MAX: usize = 200;
pub const PERSONA_MAX: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Grass,
    Water,
    Forest,
    Hill,
    Road,
    Town,
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Tile::Grass => '.',
            Tile::Water => '~',
            Tile::Forest => 'T',
            Tile::Hill => '^',
            Tile::Road => '=',
            Tile::Town => '#',
        }
    }
    pub fn walkable(self) -> bool {
        self != Tile::Water
    }
    fn ground(self) -> &'static str {
        match self {
            Tile::Grass => "open grass",
            Tile::Water => "water",
            Tile::Forest => "forest",
            Tile::Hill => "rocky hillside",
            Tile::Road => "the road",
            Tile::Town => "the town square",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NpcId(pub u32);

/// A named point on the map. The first six are seeded; the rest are founded.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub x: i32,
    pub y: i32,
    /// What can be gathered here, if anything — any word a player chose.
    pub resource: Option<String>,
    /// The skill gathering it trains.
    pub skill: Option<String>,
    pub description: String,
    pub founder: Option<PlayerId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Npc {
    pub id: NpcId,
    pub name: String,
    pub persona: String,
    pub x: i32,
    pub y: i32,
    pub creator: PlayerId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    Idle,
    /// Walking; `then` is applied on arrival (gather, bank).
    Walk {
        to: (i32, i32),
        then: Option<Box<Command>>,
    },
    /// Working a resource; `want` is the step's target, `None` means until stopped.
    Gather {
        resource: String,
        want: Option<u32>,
        got: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub inventory: Vec<(String, u32)>,
    pub bank: Vec<(String, u32)>,
    /// Experience per skill name.
    pub xp: Vec<(String, u32)>,
    pub task: Task,
    /// The rest of the current plan, applied as each step finishes.
    pub queue: VecDeque<Command>,
    /// The last plan of real steps the player ran — what `save_recipe` names.
    pub last_plan: Vec<Command>,
    pub recipes: Vec<(String, Vec<Command>)>,
    /// A recipe (name, steps) refilled into the queue whenever it runs dry.
    pub looping: Option<(String, Vec<Command>)>,
}

impl Player {
    pub fn level(&self, skill: &str) -> u32 {
        1 + ((count(&self.xp, skill) as f64) / 50.0).sqrt() as u32
    }
    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }
}

fn count(list: &[(String, u32)], key: &str) -> u32 {
    list.iter()
        .find(|(k, _)| k == key)
        .map(|(_, n)| *n)
        .unwrap_or(0)
}

fn add(list: &mut Vec<(String, u32)>, key: &str, n: u32) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 += n,
        None => list.push((key.to_string(), n)),
    }
}

/// Everything a character can be told to do. Plain data: serializable,
/// replayable, and the whole surface the model is allowed to touch.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Walk toward a place, a person, or a compass direction.
    MoveTo { target: String },
    /// Walk to the nearest source of a resource and work it: `amount` units, or until stopped.
    Gather {
        resource: String,
        amount: Option<u32>,
    },
    /// Walk to Town and deposit everything carried.
    Bank,
    /// Speak to whoever is nearby.
    Say { text: String },
    /// Do nothing, but look around.
    Look,
    /// Drop the task and the plan.
    Stop,
    /// Name the last plan so it can be run again.
    SaveRecipe { name: String },
    /// Queue a saved recipe, once or forever.
    RunRecipe { name: String, forever: bool },
    /// Found a named place where the character stands.
    FoundPlace {
        name: String,
        description: String,
        resource: Option<String>,
        skill: Option<String>,
    },
    /// Bring a character into the world where the player stands.
    CreateNpc { name: String, persona: String },
}

impl Command {
    /// Starts, replaces, or ends a task — as opposed to happening at once.
    fn is_action(&self) -> bool {
        matches!(
            self,
            Command::MoveTo { .. }
                | Command::Gather { .. }
                | Command::Bank
                | Command::RunRecipe { .. }
                | Command::Stop
        )
    }
    /// Worth remembering as a step of a recipe.
    fn is_step(&self) -> bool {
        matches!(
            self,
            Command::MoveTo { .. } | Command::Gather { .. } | Command::Bank | Command::Say { .. }
        )
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::MoveTo { target } => write!(f, "go to {target}"),
            Command::Gather {
                resource,
                amount: Some(n),
            } => write!(f, "gather {n} {resource}"),
            Command::Gather {
                resource,
                amount: None,
            } => write!(f, "gather {resource}"),
            Command::Bank => write!(f, "bank"),
            Command::Say { text } => write!(f, "say \"{text}\""),
            Command::Look => write!(f, "look"),
            Command::Stop => write!(f, "stop"),
            Command::SaveRecipe { name } => write!(f, "save recipe {name}"),
            Command::RunRecipe {
                name,
                forever: true,
            } => write!(f, "run {name} forever"),
            Command::RunRecipe {
                name,
                forever: false,
            } => write!(f, "run {name}"),
            Command::FoundPlace { name, .. } => write!(f, "found {name}"),
            Command::CreateNpc { name, .. } => write!(f, "create {name}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub tick: u64,
    /// Who did it — a player or an NPC, by name.
    pub name: String,
    pub text: String,
}

/// Words said within earshot of an NPC, waiting for a voice. The world cannot
/// speak for an NPC (that takes a model); it records who was addressed and
/// hands the speech to whoever hosts it via `take_speeches`.
#[derive(Clone, Debug, PartialEq)]
pub struct Speech {
    pub tick: u64,
    pub speaker: PlayerId,
    pub listener: NpcId,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    pub seed: u64,
    pub tick: u64,
    tiles: Vec<Tile>,
    pub places: Vec<Place>,
    pub npcs: Vec<Npc>,
    pub players: Vec<Player>,
    pub events: Vec<Event>,
    speeches: Vec<Speech>,
    next_id: u32,
    next_npc: u32,
}

/// xorshift64*: one integer of state, no floats, reproducible anywhere.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u32) -> i32 {
        (self.next() % n as u64) as i32
    }
}

const DIRS: [(&str, (i32, i32)); 8] = [
    ("north", (0, -1)),
    ("south", (0, 1)),
    ("east", (1, 0)),
    ("west", (-1, 0)),
    ("northeast", (1, -1)),
    ("northwest", (-1, -1)),
    ("southeast", (1, 1)),
    ("southwest", (-1, 1)),
];

fn near(ax: i32, ay: i32, bx: i32, by: i32) -> bool {
    (ax - bx).abs() <= 1 && (ay - by).abs() <= 1
}

/// Collapse whitespace, trim, cap the length.
fn tidy(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// A proper name: letters, digits, spaces, apostrophes and hyphens.
fn clean_name(s: &str) -> Result<String, String> {
    let s = tidy(s, NAME_MAX);
    if s.chars().count() < 2 {
        return Err("that name is too short".into());
    }
    if !s
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '\'' | '-'))
    {
        return Err("a name is letters, digits, spaces, apostrophes and hyphens".into());
    }
    Ok(s)
}

/// A resource or skill word: lowercase letters and spaces.
fn clean_word(s: &str) -> Result<String, String> {
    let s = tidy(s, WORD_MAX).to_lowercase();
    if s.chars().count() < 2 || !s.chars().all(|c| c.is_alphabetic() || c == ' ') {
        return Err(format!("'{s}' is not a resource word (lowercase letters)"));
    }
    Ok(s)
}

fn singular(s: &str) -> &str {
    if s.ends_with("ss") {
        return s;
    }
    s.strip_suffix("es")
        .filter(|b| b.ends_with('s') || b.ends_with('x') || b.ends_with("sh") || b.ends_with("ch"))
        .or_else(|| s.strip_suffix('s'))
        .unwrap_or(s)
}

impl World {
    pub fn new(seed: u64) -> World {
        let mut rng = Rng(seed | 1);
        let mut tiles = vec![Tile::Grass; (W * H) as usize];
        let at = |x: i32, y: i32| (y * W + x) as usize;

        // A river down the left third, wandering one tile at a time.
        let mut rx = 7 + rng.below(3);
        for y in 0..H {
            tiles[at(rx, y)] = Tile::Water;
            if rx + 1 < W && rng.below(3) == 0 {
                tiles[at(rx + 1, y)] = Tile::Water;
            }
            rx = (rx + rng.below(3) - 1).clamp(4, 11);
        }
        // A forest in the east, hills to the north, the town in the middle.
        let blob = |tiles: &mut Vec<Tile>, rng: &mut Rng, cx: i32, cy: i32, r: i32, t: Tile| {
            for y in (cy - r).max(0)..=(cy + r).min(H - 1) {
                for x in (cx - r).max(0)..=(cx + r).min(W - 1) {
                    let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
                    if d2 <= r * r - rng.below(r as u32 + 1) && tiles[at(x, y)] == Tile::Grass {
                        tiles[at(x, y)] = t;
                    }
                }
            }
        };
        let forest = (24 + rng.below(3), 10 + rng.below(3));
        let hill = (17 + rng.below(4), 2 + rng.below(2));
        let quarry = (26 + rng.below(3), 2 + rng.below(2));
        blob(&mut tiles, &mut rng, forest.0, forest.1, 4, Tile::Forest);
        blob(&mut tiles, &mut rng, hill.0, hill.1, 3, Tile::Hill);
        blob(&mut tiles, &mut rng, quarry.0, quarry.1, 2, Tile::Hill);
        let town = (16, 9);
        for y in town.1 - 1..=town.1 + 1 {
            for x in town.0 - 2..=town.0 + 2 {
                tiles[at(x, y)] = Tile::Town;
            }
        }
        // A road from the town gate north to the hill.
        for y in hill.1 + 2..town.1 - 1 {
            let x =
                town.0 + (hill.0 - town.0) * (town.1 - 1 - y) / (town.1 - 1 - hill.1 - 2).max(1);
            if tiles[at(x, y)] == Tile::Grass {
                tiles[at(x, y)] = Tile::Road;
            }
        }
        // The ford: the river is crossable on the town's row, and that is where
        // the fishing is. Without it the west bank is an island.
        let mut fishing = (12, town.1);
        for x in 0..W {
            if tiles[at(x, town.1)] == Tile::Water {
                tiles[at(x, town.1)] = Tile::Road;
                fishing = (x + 1, town.1);
            }
        }
        let creek = (5, 14 + rng.below(3));
        let seeded =
            |name: &str, (x, y): (i32, i32), res: Option<(&str, &str)>, desc: &str| Place {
                name: name.into(),
                x,
                y,
                resource: res.map(|(r, _)| r.into()),
                skill: res.map(|(_, s)| s.into()),
                description: desc.into(),
                founder: None,
            };
        let mut places = vec![
            seeded(
                "Town",
                town,
                None,
                "A square, a well, and the bank. Everything starts here.",
            ),
            seeded(
                "Iron Hill",
                hill,
                Some(("iron", "mining")),
                "Red rock and old shafts. The pick rings.",
            ),
            seeded(
                "Old Forest",
                forest,
                Some(("wood", "woodcutting")),
                "Oaks older than the town. Axes welcome.",
            ),
            seeded(
                "Quarry",
                quarry,
                Some(("stone", "mining")),
                "A cut face of grey stone.",
            ),
            seeded(
                "River Ford",
                fishing,
                Some(("fish", "fishing")),
                "Shallow water over stones; the fish gather in the eddies.",
            ),
            seeded(
                "Gold Creek",
                creek,
                Some(("gold", "mining")),
                "A thin creek on the far bank. Flecks in the pan.",
            ),
        ];
        for p in &mut places {
            // Every place stands on walkable ground.
            if !tiles[at(p.x, p.y)].walkable() {
                tiles[at(p.x, p.y)] = Tile::Grass;
            }
        }
        World {
            seed,
            tick: 0,
            tiles,
            places,
            npcs: Vec::new(),
            players: Vec::new(),
            events: Vec::new(),
            speeches: Vec::new(),
            next_id: 1,
            next_npc: 1,
        }
    }

    pub fn tile(&self, x: i32, y: i32) -> Tile {
        if x < 0 || y < 0 || x >= W || y >= H {
            Tile::Water
        } else {
            self.tiles[(y * W + x) as usize]
        }
    }

    /// A new character appears in town.
    pub fn join(&mut self, name: impl Into<String>) -> PlayerId {
        let id = PlayerId(self.next_id);
        self.next_id += 1;
        let town = &self.places[0];
        let n = self.players.len() as i32;
        let name = name.into();
        self.players.push(Player {
            id,
            name: name.clone(),
            x: town.x + (n % 5) - 2,
            y: town.y + (n / 5) % 3 - 1,
            inventory: Vec::new(),
            bank: Vec::new(),
            xp: Vec::new(),
            task: Task::Idle,
            queue: VecDeque::new(),
            last_plan: Vec::new(),
            recipes: Vec::new(),
            looping: None,
        });
        self.note(&name, "arrived in Town");
        id
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }
    fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }
    fn name_of(&self, id: PlayerId) -> String {
        self.player(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "someone".into())
    }

    /// Loose lookup: exact, then substring, then any shared word ("the hill").
    pub fn place(&self, name: &str) -> Option<&Place> {
        let q = name.trim().to_ascii_lowercase();
        let q = q.trim_start_matches("the ").to_string();
        if q.is_empty() {
            return None;
        }
        self.places
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(&q))
            .or_else(|| {
                self.places
                    .iter()
                    .find(|p| p.name.to_ascii_lowercase().contains(&q))
            })
            .or_else(|| {
                // A shared word, as long as it is a telling one: "the hill", not "old".
                self.places.iter().find(|p| {
                    p.name.split_whitespace().any(|w| {
                        let w = w.to_ascii_lowercase();
                        w.len() >= 4
                            && !matches!(w.as_str(), "old" | "new" | "the")
                            && q.split_whitespace().any(|qw| qw == w)
                    })
                })
            })
    }
    pub fn place_at(&self, x: i32, y: i32) -> Option<&Place> {
        self.places.iter().find(|p| near(p.x, p.y, x, y))
    }
    /// The nearest place yielding `resource` (singular or plural).
    fn source_of(&self, resource: &str, from: (i32, i32)) -> Option<&Place> {
        let want = singular(resource);
        self.places
            .iter()
            .filter(|p| p.resource.as_deref().map(singular) == Some(want))
            .min_by_key(|p| (p.x - from.0).abs() + (p.y - from.1).abs())
    }
    pub fn npcs_near(&self, who: PlayerId) -> Vec<&Npc> {
        match self.player(who) {
            Some(p) => self
                .npcs
                .iter()
                .filter(|n| near(n.x, n.y, p.x, p.y))
                .collect(),
            None => Vec::new(),
        }
    }
    pub fn npc(&self, id: NpcId) -> Option<&Npc> {
        self.npcs.iter().find(|n| n.id == id)
    }
    /// An NPC spoke (the voice is produced outside the world).
    pub fn npc_says(&mut self, id: NpcId, text: &str) {
        if let Some(n) = self.npc(id) {
            let name = n.name.clone();
            self.note(&name, format!("says \"{}\"", tidy(text, TEXT_MAX)));
        }
    }
    /// Everything said to an NPC since the last call, oldest first.
    pub fn take_speeches(&mut self) -> Vec<Speech> {
        std::mem::take(&mut self.speeches)
    }
    /// The same, without taking: for hosts that answer by appending to a ledger.
    pub fn speeches(&self) -> &[Speech] {
        &self.speeches
    }
    /// An NPC has answered the speech made to it at `for_tick`: forget it.
    pub fn answer_speech(&mut self, npc: NpcId, for_tick: u64) {
        self.speeches
            .retain(|s| !(s.listener == npc && s.tick == for_tick));
    }

    fn note(&mut self, name: &str, text: impl Into<String>) {
        self.events.push(Event {
            tick: self.tick,
            name: name.to_string(),
            text: text.into(),
        });
        if self.events.len() > 200 {
            self.events.drain(..100);
        }
    }

    /// One command. `Ok` is the acknowledgement; `Err` is a refusal with a reason.
    pub fn apply(&mut self, who: PlayerId, cmd: &Command) -> Result<String, String> {
        self.plan(who, vec![cmd.clone()])
    }

    /// A plan: several commands in order. Actions replace whatever the
    /// character was doing and run one after another; instant commands
    /// (say, look, save, found, create) happen now. The acknowledgements
    /// come back newline-joined; a refused step is prefixed with `x`.
    pub fn plan(&mut self, who: PlayerId, cmds: Vec<Command>) -> Result<String, String> {
        if cmds.is_empty() {
            return Err("nothing to do".into());
        }
        if self.player(who).is_none() {
            return Err("no such player".into());
        }
        let mut acks = Vec::new();
        if cmds.iter().any(Command::is_action) {
            let steps: Vec<Command> = cmds.iter().filter(|c| c.is_step()).cloned().collect();
            let p = self.player_mut(who).unwrap();
            p.task = Task::Idle;
            p.queue.clear();
            p.looping = None;
            if !steps.is_empty() {
                p.last_plan = steps;
            }
            p.queue.extend(cmds.into_iter().take(MAX_QUEUE));
            acks.extend(self.advance(who));
        } else {
            for c in &cmds {
                match self.apply_one(who, c) {
                    Ok(a) => acks.push(a),
                    Err(e) => acks.push(format!("x {e}")),
                }
            }
        }
        Ok(acks.join("\n"))
    }

    /// Run queued steps until one starts a task (or the queue is dry).
    fn advance(&mut self, who: PlayerId) -> Vec<String> {
        let mut acks = Vec::new();
        let mut refilled = false;
        loop {
            let p = self.player_mut(who).unwrap();
            if p.task != Task::Idle {
                break;
            }
            let cmd = match p.queue.pop_front() {
                Some(c) => c,
                None => match &p.looping {
                    Some((_, l)) if !refilled && !l.is_empty() => {
                        let l = l.clone();
                        p.queue.extend(l);
                        refilled = true;
                        continue;
                    }
                    _ => break,
                },
            };
            match self.apply_one(who, &cmd) {
                Ok(a) => acks.push(a),
                Err(e) => {
                    let name = self.name_of(who);
                    self.note(&name, format!("plan stopped: {e}"));
                    let p = self.player_mut(who).unwrap();
                    p.queue.clear();
                    p.looping = None;
                    acks.push(format!("x {e}"));
                    break;
                }
            }
        }
        acks
    }

    fn apply_one(&mut self, who: PlayerId, cmd: &Command) -> Result<String, String> {
        let (px, py, name) = {
            let p = self.player(who).ok_or("no such player")?;
            (p.x, p.y, p.name.clone())
        };
        match cmd {
            Command::MoveTo { target } => {
                let (dest, label) = self.resolve_target(who, target).ok_or_else(|| {
                    format!(
                        "there is no '{target}' to walk to. Places: {}. Or a person's name, or a compass direction.",
                        self.places.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                    )
                })?;
                if dest == (px, py) {
                    return Ok(format!("{name} is already at {label}."));
                }
                self.player_mut(who).unwrap().task = Task::Walk {
                    to: dest,
                    then: None,
                };
                self.note(&name, format!("set out for {label}"));
                Ok(format!("{name} sets out for {label}."))
            }
            Command::Gather { resource, amount } => {
                let res = clean_word(resource)?;
                let src = self.source_of(&res, (px, py)).cloned().ok_or_else(|| {
                    format!(
                        "nothing in this world yields {res}. Known: {}. Someone could found a place that does.",
                        self.places
                            .iter()
                            .filter_map(|p| p.resource.as_deref())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
                let res = src.resource.clone().unwrap();
                let here = near(src.x, src.y, px, py);
                let me = self.player_mut(who).unwrap();
                if here {
                    me.task = Task::Gather {
                        resource: res.clone(),
                        want: *amount,
                        got: 0,
                    };
                    self.note(&name, format!("began gathering {res} at {}", src.name));
                    Ok(format!("{name} begins gathering {res} at {}.", src.name))
                } else {
                    me.task = Task::Walk {
                        to: (src.x, src.y),
                        then: Some(Box::new(Command::Gather {
                            resource: res.clone(),
                            amount: *amount,
                        })),
                    };
                    self.note(&name, format!("set out for {} to gather {res}", src.name));
                    Ok(format!("{name} heads for {} to gather {res}.", src.name))
                }
            }
            Command::Bank => {
                let town = self.places[0].clone();
                if near(town.x, town.y, px, py) {
                    let me = self.player_mut(who).unwrap();
                    let carried = std::mem::take(&mut me.inventory);
                    if carried.is_empty() {
                        return Ok(format!("{name} has nothing to deposit."));
                    }
                    let list: Vec<String> =
                        carried.iter().map(|(r, n)| format!("{n} {r}")).collect();
                    for (r, n) in &carried {
                        add(&mut me.bank, r, *n);
                    }
                    let summary = list.join(", ");
                    self.note(&name, format!("banked {summary}"));
                    Ok(format!("{name} deposits {summary}."))
                } else {
                    self.player_mut(who).unwrap().task = Task::Walk {
                        to: (town.x, town.y),
                        then: Some(Box::new(Command::Bank)),
                    };
                    self.note(&name, "set out for Town to bank");
                    Ok(format!("{name} heads to Town to bank."))
                }
            }
            Command::Say { text } => {
                let text = tidy(text, TEXT_MAX);
                if text.is_empty() {
                    return Err("nothing to say".into());
                }
                self.note(&name, format!("says \"{text}\""));
                if let Some(n) = self.npcs.iter().find(|n| near(n.x, n.y, px, py)) {
                    let speech = Speech {
                        tick: self.tick,
                        speaker: who,
                        listener: n.id,
                        text: text.clone(),
                    };
                    self.speeches.push(speech);
                }
                Ok(format!("{name} says \"{text}\""))
            }
            Command::Look => Ok(self.describe(who)),
            Command::Stop => {
                // Ends the activity and any repeat. The queue is the plan this
                // command is part of, so a plan may begin with stop and go on.
                let me = self.player_mut(who).unwrap();
                me.task = Task::Idle;
                me.looping = None;
                self.note(&name, "stopped");
                Ok(format!("{name} stops."))
            }
            Command::SaveRecipe { name: rname } => {
                let rname = clean_name(rname)?;
                let me = self.player_mut(who).unwrap();
                if me.last_plan.is_empty() {
                    return Err("nothing to save yet: run a plan first, then name it".into());
                }
                let steps = me.last_plan.clone();
                let shown = steps
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                match me
                    .recipes
                    .iter_mut()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&rname))
                {
                    Some(slot) => slot.1 = steps,
                    None => {
                        if me.recipes.len() >= MAX_RECIPES {
                            return Err(format!(
                                "{MAX_RECIPES} recipes is the limit; save over one"
                            ));
                        }
                        me.recipes.push((rname.clone(), steps));
                    }
                }
                Ok(format!("{name} saves recipe '{rname}': {shown}."))
            }
            Command::RunRecipe {
                name: rname,
                forever,
            } => {
                let q = rname.trim().to_ascii_lowercase();
                let me = self.player_mut(who).unwrap();
                let found = me
                    .recipes
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&q))
                    .or_else(|| {
                        me.recipes
                            .iter()
                            .find(|(n, _)| n.to_ascii_lowercase().contains(&q))
                    })
                    .cloned();
                let (rname, steps) = found.ok_or_else(|| {
                    if me.recipes.is_empty() {
                        "no recipes yet: run a plan, then 'save this as <name>'".to_string()
                    } else {
                        format!(
                            "no recipe '{rname}'. Recipes: {}",
                            me.recipes
                                .iter()
                                .map(|(n, _)| n.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                })?;
                if *forever && !steps.iter().any(Command::is_action) {
                    return Err("that recipe has no steps worth repeating".into());
                }
                me.queue.extend(steps.iter().cloned());
                if *forever {
                    me.looping = Some((rname.clone(), steps));
                }
                self.note(
                    &name,
                    format!("runs '{rname}'{}", if *forever { " on repeat" } else { "" }),
                );
                Ok(format!(
                    "{name} runs '{rname}'{}.",
                    if *forever { " on repeat" } else { "" }
                ))
            }
            Command::FoundPlace {
                name: pname,
                description,
                resource,
                skill,
            } => {
                let pname = clean_name(pname)?;
                if self
                    .places
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(&pname))
                {
                    return Err(format!("there is already a place called {pname}"));
                }
                if let Some(p) = self
                    .places
                    .iter()
                    .find(|p| (p.x - px).abs() <= 2 && (p.y - py).abs() <= 2)
                {
                    return Err(format!(
                        "too close to {}; walk a few tiles away first",
                        p.name
                    ));
                }
                let mine = self
                    .places
                    .iter()
                    .filter(|p| p.founder == Some(who))
                    .count();
                if mine >= MAX_PLACES_PER_PLAYER {
                    return Err(format!(
                        "{MAX_PLACES_PER_PLAYER} places founded is the limit"
                    ));
                }
                let resource = resource
                    .as_deref()
                    .filter(|r| !r.trim().is_empty())
                    .map(clean_word)
                    .transpose()?;
                let skill = match (&resource, skill) {
                    (None, _) => None,
                    (Some(_), Some(s)) if !s.trim().is_empty() => Some(clean_word(s)?),
                    (Some(_), _) => Some("gathering".to_string()),
                };
                let description = tidy(description, TEXT_MAX);
                self.places.push(Place {
                    name: pname.clone(),
                    x: px,
                    y: py,
                    resource: resource.clone(),
                    skill,
                    description,
                    founder: Some(who),
                });
                self.note(&name, format!("founded {pname}"));
                Ok(match resource {
                    Some(r) => format!("{name} founds {pname}. It yields {r}."),
                    None => format!("{name} founds {pname}."),
                })
            }
            Command::CreateNpc {
                name: nname,
                persona,
            } => {
                let nname = clean_name(nname)?;
                if self
                    .npcs
                    .iter()
                    .any(|n| n.name.eq_ignore_ascii_case(&nname))
                    || self
                        .players
                        .iter()
                        .any(|p| p.name.eq_ignore_ascii_case(&nname))
                {
                    return Err(format!("someone called {nname} already exists"));
                }
                let mine = self.npcs.iter().filter(|n| n.creator == who).count();
                if mine >= MAX_NPCS_PER_PLAYER {
                    return Err(format!(
                        "{MAX_NPCS_PER_PLAYER} characters created is the limit"
                    ));
                }
                let persona = tidy(persona, PERSONA_MAX);
                if persona.chars().count() < 8 {
                    return Err("say who they are: a persona of a sentence or three".into());
                }
                let id = NpcId(self.next_npc);
                self.next_npc += 1;
                self.npcs.push(Npc {
                    id,
                    name: nname.clone(),
                    persona,
                    x: px,
                    y: py,
                    creator: who,
                });
                self.note(&name, format!("brought {nname} into the world"));
                Ok(format!("{nname} is here now, beside {name}."))
            }
        }
    }

    /// A place, a person, or a compass direction (five tiles that way, walked
    /// back onto land). Returns the destination and how to name it.
    fn resolve_target(&self, who: PlayerId, target: &str) -> Option<((i32, i32), String)> {
        let from = self.player(who).map(|p| p.pos())?;
        // People by exact name first, so "Old Wren" is not "Old Forest".
        let q = target.trim().to_ascii_lowercase();
        if let Some(n) = self.npcs.iter().find(|n| n.name.to_ascii_lowercase() == q) {
            return Some(((n.x, n.y), n.name.clone()));
        }
        if let Some(p) = self
            .players
            .iter()
            .find(|p| p.id != who && p.name.to_ascii_lowercase() == q)
        {
            return Some(((p.x, p.y), p.name.clone()));
        }
        if let Some(p) = self.place(target) {
            return Some(((p.x, p.y), p.name.clone()));
        }
        let t = q
            .trim_start_matches("go ")
            .trim_start_matches("to ")
            .trim_start_matches("the ")
            .trim();
        let t = match t {
            "n" => "north",
            "s" => "south",
            "e" => "east",
            "w" => "west",
            "ne" => "northeast",
            "nw" => "northwest",
            "se" => "southeast",
            "sw" => "southwest",
            other => other,
        };
        if t.is_empty() {
            return None;
        }
        let (dir, (dx, dy)) = DIRS
            .iter()
            .find(|(n, _)| *n == t)
            .or_else(|| DIRS.iter().find(|(n, _)| n.starts_with(t)))?;
        for step in (1..=5).rev() {
            let x = (from.0 + dx * step).clamp(0, W - 1);
            let y = (from.1 + dy * step).clamp(0, H - 1);
            if self.tile(x, y).walkable() {
                return Some(((x, y), format!("{step} tiles {dir}")));
            }
        }
        None
    }

    /// One tick: every character takes one step or one swing, and plans advance.
    pub fn step(&mut self) {
        self.tick += 1;
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            let p = self.player(id).unwrap().clone();
            match p.task {
                Task::Idle => {}
                Task::Walk { to, then } => {
                    let arrived =
                        (p.x, p.y) == to || (then.is_some() && near(p.x, p.y, to.0, to.1));
                    if arrived {
                        self.player_mut(id).unwrap().task = Task::Idle;
                        match then {
                            Some(next) => {
                                if let Err(e) = self.apply_one(id, &next) {
                                    self.note(&p.name, format!("plan stopped: {e}"));
                                    let me = self.player_mut(id).unwrap();
                                    me.queue.clear();
                                    me.looping = None;
                                }
                            }
                            None => {
                                let label = self.label(to);
                                self.note(&p.name, format!("arrived at {label}"));
                            }
                        }
                    } else if let Some(next) = self.path_step((p.x, p.y), to) {
                        let me = self.player_mut(id).unwrap();
                        me.x = next.0;
                        me.y = next.1;
                    } else {
                        let me = self.player_mut(id).unwrap();
                        me.task = Task::Idle;
                        me.queue.clear();
                        me.looping = None;
                        self.note(&p.name, "could not find a way there");
                    }
                }
                Task::Gather {
                    resource,
                    want,
                    got,
                } => {
                    let spot = self
                        .places
                        .iter()
                        .find(|pl| {
                            near(pl.x, pl.y, p.x, p.y)
                                && pl.resource.as_deref() == Some(resource.as_str())
                        })
                        .cloned();
                    match spot {
                        Some(spot) => {
                            let skill = spot.skill.clone().unwrap_or_else(|| "gathering".into());
                            let me = self.player_mut(id).unwrap();
                            let before = me.level(&skill);
                            add(&mut me.inventory, &resource, 1);
                            add(&mut me.xp, &skill, 10);
                            let after = me.level(&skill);
                            let got = got + 1;
                            me.task = Task::Gather {
                                resource: resource.clone(),
                                want,
                                got,
                            };
                            if after > before {
                                self.note(&p.name, format!("reached {skill} level {after}"));
                            }
                            if want.is_some_and(|w| got >= w) {
                                self.player_mut(id).unwrap().task = Task::Idle;
                                self.note(&p.name, format!("gathered {got} {resource}"));
                            }
                        }
                        None => {
                            let me = self.player_mut(id).unwrap();
                            me.task = Task::Idle;
                            me.queue.clear();
                            me.looping = None;
                            self.note(&p.name, format!("found no {resource} here"));
                        }
                    }
                }
            }
            // Whatever just finished, the plan goes on.
            let waiting = {
                let me = self.player(id).unwrap();
                me.task == Task::Idle && (!me.queue.is_empty() || me.looping.is_some())
            };
            if waiting {
                self.advance(id);
            }
        }
    }

    /// Breadth-first search over walkable tiles: the next tile toward `to`.
    fn path_step(&self, from: (i32, i32), to: (i32, i32)) -> Option<(i32, i32)> {
        if from == to {
            return None;
        }
        let idx = |(x, y): (i32, i32)| (y * W + x) as usize;
        let mut prev = vec![u32::MAX; (W * H) as usize];
        let mut queue = VecDeque::new();
        prev[idx(from)] = idx(from) as u32;
        queue.push_back(from);
        let mut found = false;
        while let Some(cur) = queue.pop_front() {
            if cur == to {
                found = true;
                break;
            }
            for (_, (dx, dy)) in DIRS.iter().take(4) {
                let n = (cur.0 + dx, cur.1 + dy);
                if n.0 < 0 || n.1 < 0 || n.0 >= W || n.1 >= H {
                    continue;
                }
                if !self.tile(n.0, n.1).walkable() || prev[idx(n)] != u32::MAX {
                    continue;
                }
                prev[idx(n)] = idx(cur) as u32;
                queue.push_back(n);
            }
        }
        if !found {
            return None;
        }
        let mut cur = to;
        loop {
            let p = prev[idx(cur)] as usize;
            let pp = ((p % W as usize) as i32, (p / W as usize) as i32);
            if pp == from {
                return Some(cur);
            }
            cur = pp;
        }
    }

    fn label(&self, at: (i32, i32)) -> String {
        self.place_at(at.0, at.1)
            .map(|p| p.name.clone())
            .or_else(|| {
                self.npcs
                    .iter()
                    .find(|n| (n.x, n.y) == at)
                    .map(|n| n.name.clone())
            })
            .unwrap_or(format!("({},{})", at.0, at.1))
    }

    /// The text view: what this character knows right now. This is the whole
    /// context the pilot gives the model, so it is short and complete.
    pub fn describe(&self, who: PlayerId) -> String {
        let Some(p) = self.player(who) else {
            return "You are nobody.".into();
        };
        let mut s = String::new();
        let here = self.place_at(p.x, p.y);
        match here {
            Some(pl) => s.push_str(&format!(
                "You are {} at {} ({},{}), on {}.",
                p.name,
                pl.name,
                p.x,
                p.y,
                self.tile(p.x, p.y).ground()
            )),
            None => s.push_str(&format!(
                "You are {} at ({},{}), on {}.",
                p.name,
                p.x,
                p.y,
                self.tile(p.x, p.y).ground()
            )),
        }
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
            } => format!("gathering {resource} ({got} so far, until stopped)"),
        };
        s.push_str(&format!(" Tick {}. You are {doing}.\n", self.tick));
        if !p.queue.is_empty() || p.looping.is_some() {
            let mut then: Vec<String> = p.queue.iter().map(|c| c.to_string()).collect();
            if let Some((rname, _)) = &p.looping {
                then.push(format!("repeat '{rname}'"));
            }
            s.push_str(&format!("Then: {}\n", then.join(", ")));
        }
        if let Some(pl) = here {
            let founder = pl
                .founder
                .map(|f| format!(" Founded by {}.", self.name_of(f)))
                .unwrap_or_default();
            s.push_str(&format!(
                "Here: {} — \"{}\"{founder}\n",
                pl.name, pl.description
            ));
        }
        s.push_str("Places: ");
        let mut first = true;
        for pl in &self.places {
            if !first {
                s.push_str(", ");
            }
            first = false;
            let d = (pl.x - p.x).abs().max((pl.y - p.y).abs());
            let where_ = if d == 0 {
                "here".to_string()
            } else {
                format!("{d} {}", compass(p.x, p.y, pl.x, pl.y))
            };
            match (&pl.resource, &pl.skill) {
                (Some(r), Some(sk)) => s.push_str(&format!("{} ({where_}, {r}/{sk})", pl.name)),
                (Some(r), None) => s.push_str(&format!("{} ({where_}, {r})", pl.name)),
                _ => s.push_str(&format!("{} ({where_})", pl.name)),
            }
        }
        s.push('\n');
        let mut people: Vec<String> = self
            .players
            .iter()
            .filter(|o| o.id != who)
            .map(|o| {
                let d = (o.x - p.x).abs().max((o.y - p.y).abs());
                let doing = match &o.task {
                    Task::Idle => "idle".to_string(),
                    Task::Walk { .. } => "walking".to_string(),
                    Task::Gather { resource, .. } => format!("gathering {resource}"),
                };
                if d == 0 {
                    format!("{} (here, {doing})", o.name)
                } else {
                    format!("{} ({d} {}, {doing})", o.name, compass(p.x, p.y, o.x, o.y))
                }
            })
            .collect();
        for n in &self.npcs {
            let d = (n.x - p.x).abs().max((n.y - p.y).abs());
            if d <= 1 {
                people.push(format!("{} (NPC, here)", n.name));
            } else {
                people.push(format!(
                    "{} (NPC, {d} {})",
                    n.name,
                    compass(p.x, p.y, n.x, n.y)
                ));
            }
        }
        s.push_str(&format!(
            "People: {}\n",
            if people.is_empty() {
                "nobody".to_string()
            } else {
                people.join(", ")
            }
        ));
        let inv: Vec<String> = p
            .inventory
            .iter()
            .map(|(r, n)| format!("{n} {r}"))
            .collect();
        let bank: Vec<String> = p.bank.iter().map(|(r, n)| format!("{n} {r}")).collect();
        s.push_str(&format!(
            "Carrying: {}. Bank: {}.\n",
            if inv.is_empty() {
                "nothing".to_string()
            } else {
                inv.join(", ")
            },
            if bank.is_empty() {
                "empty".to_string()
            } else {
                bank.join(", ")
            }
        ));
        if !p.xp.is_empty() {
            let skills: Vec<String> =
                p.xp.iter()
                    .map(|(sk, _)| format!("{sk} {}", p.level(sk)))
                    .collect();
            s.push_str(&format!("Skills: {}\n", skills.join(", ")));
        }
        if !p.recipes.is_empty() {
            let rs: Vec<String> = p
                .recipes
                .iter()
                .map(|(n, steps)| {
                    format!(
                        "{n} = {}",
                        steps
                            .iter()
                            .map(|c| c.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect();
            s.push_str(&format!("Recipes: {}\n", rs.join("; ")));
        }
        let recent: Vec<String> = self
            .events
            .iter()
            .rev()
            .take(6)
            .map(|e| format!("[t{}] {} {}", e.tick, e.name, e.text))
            .collect();
        if !recent.is_empty() {
            s.push_str("Recently: ");
            s.push_str(&recent.into_iter().rev().collect::<Vec<_>>().join("; "));
            s.push('\n');
        }
        s
    }

    /// The display, for a terminal: tiles, places `*`, players as their
    /// initial, NPCs as their lowercase initial.
    pub fn ascii(&self) -> String {
        let mut grid: Vec<Vec<char>> = (0..H)
            .map(|y| (0..W).map(|x| self.tile(x, y).glyph()).collect())
            .collect();
        for p in &self.places {
            grid[p.y as usize][p.x as usize] = '*';
        }
        for n in &self.npcs {
            grid[n.y as usize][n.x as usize] =
                n.name.chars().next().unwrap_or('n').to_ascii_lowercase();
        }
        for p in &self.players {
            grid[p.y as usize][p.x as usize] =
                p.name.chars().next().unwrap_or('@').to_ascii_uppercase();
        }
        let mut s = String::new();
        for row in grid {
            s.extend(row);
            s.push('\n');
        }
        s
    }
}

fn compass(fx: i32, fy: i32, tx: i32, ty: i32) -> &'static str {
    let (dx, dy) = (tx - fx, ty - fy);
    if dx == 0 && dy == 0 {
        return "here";
    }
    let horiz = dx.abs() >= dy.abs() * 2;
    let vert = dy.abs() >= dx.abs() * 2;
    match (dx.signum(), dy.signum()) {
        (_, -1) if vert => "N",
        (_, 1) if vert => "S",
        (1, _) if horiz => "E",
        (-1, _) if horiz => "W",
        (1, -1) => "NE",
        (-1, -1) => "NW",
        (1, 1) => "SE",
        (-1, 1) => "SW",
        (1, 0) => "E",
        (-1, 0) => "W",
        (0, -1) => "N",
        _ => "S",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gather(r: &str, n: Option<u32>) -> Command {
        Command::Gather {
            resource: r.into(),
            amount: n,
        }
    }

    #[test]
    fn worlds_are_reproducible_and_places_are_reachable() {
        assert_eq!(World::new(7).ascii(), World::new(7).ascii());
        let mut w = World::new(7);
        let me = w.join("Kyle");
        for pl in w.places.clone() {
            w.apply(
                me,
                &Command::MoveTo {
                    target: pl.name.clone(),
                },
            )
            .unwrap();
            for _ in 0..80 {
                w.step();
            }
            assert_eq!(
                w.player(me).unwrap().pos(),
                (pl.x, pl.y),
                "did not reach {}",
                pl.name
            );
        }
    }

    #[test]
    fn a_plan_runs_step_by_step_and_banks() {
        let mut w = World::new(3);
        let me = w.join("Ann");
        let ack = w
            .plan(
                me,
                vec![
                    gather("wood", Some(3)),
                    Command::Bank,
                    Command::Say {
                        text: "done".into(),
                    },
                ],
            )
            .unwrap();
        assert!(ack.contains("heads for Old Forest"), "{ack}");
        for _ in 0..120 {
            w.step();
        }
        let p = w.player(me).unwrap();
        assert_eq!(p.task, Task::Idle);
        assert!(p.inventory.is_empty());
        assert_eq!(count(&p.bank, "wood"), 3);
        assert!(w.events.iter().any(|e| e.text == "says \"done\""));
        assert_eq!(p.last_plan.len(), 3);
    }

    #[test]
    fn recipes_save_and_loop() {
        let mut w = World::new(3);
        let me = w.join("Ann");
        assert!(w
            .apply(me, &Command::SaveRecipe { name: "run".into() })
            .unwrap()
            .starts_with("x nothing to save"));
        w.plan(me, vec![gather("iron", Some(2)), Command::Bank])
            .unwrap();
        // Saving mid-plan does not interrupt it.
        assert!(w
            .apply(
                me,
                &Command::SaveRecipe {
                    name: "ironrun".into()
                }
            )
            .unwrap()
            .contains("saves recipe"));
        assert!(matches!(w.player(me).unwrap().task, Task::Walk { .. }));
        w.apply(
            me,
            &Command::RunRecipe {
                name: "iron".into(),
                forever: true,
            },
        )
        .unwrap();
        for _ in 0..200 {
            w.step();
        }
        let p = w.player(me).unwrap();
        assert!(count(&p.bank, "iron") >= 6, "looped: {:?}", p.bank);
        assert!(p.looping.is_some());
        w.apply(me, &Command::Stop).unwrap();
        let p = w.player(me).unwrap();
        assert!(p.looping.is_none() && p.queue.is_empty() && p.task == Task::Idle);
        assert!(w
            .describe(me)
            .contains("Recipes: ironrun = gather 2 iron, bank"));
    }

    #[test]
    fn speech_near_an_npc_waits_for_a_voice_and_stop_keeps_the_rest_of_a_plan() {
        let mut w = World::new(5);
        let me = w.join("Kyle");
        w.apply(
            me,
            &Command::CreateNpc {
                name: "Wren".into(),
                persona: "A forager who talks to birds.".into(),
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
        let s = w.take_speeches();
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].speaker, s[0].text.as_str()), (me, "hello"));
        assert!(w.take_speeches().is_empty());
        // A plan that begins with stop still runs its later steps.
        let ack = w
            .plan(
                me,
                vec![Command::Stop, gather("iron", Some(1)), Command::Bank],
            )
            .unwrap();
        assert!(
            ack.contains("stops") && ack.contains("heads for Iron Hill"),
            "{ack}"
        );
        assert!(w.describe(me).contains("Then: bank"));
        // Out of earshot, speech is just speech.
        for _ in 0..30 {
            w.step();
        }
        w.apply(me, &Command::Say { text: "far".into() }).unwrap();
        assert!(w.take_speeches().is_empty());
    }

    #[test]
    fn players_found_places_with_any_resource_and_create_npcs() {
        let mut w = World::new(5);
        let me = w.join("Kyle");
        assert!(w
            .apply(me, &gather("mushrooms", None))
            .unwrap()
            .starts_with("x nothing in this world yields"));
        let found = Command::FoundPlace {
            name: "Damp Hollow".into(),
            description: "Mushrooms under every log.".into(),
            resource: Some("Mushrooms".into()),
            skill: Some("foraging".into()),
        };
        assert!(w
            .apply(me, &found)
            .unwrap()
            .starts_with("x too close to Town"));
        w.apply(
            me,
            &Command::MoveTo {
                target: "east".into(),
            },
        )
        .unwrap();
        for _ in 0..10 {
            w.step();
        }
        assert!(w.apply(me, &found).unwrap().contains("founds Damp Hollow"));
        assert!(w
            .apply(me, &found)
            .unwrap()
            .starts_with("x there is already"));
        w.apply(me, &gather("mushroom", Some(2))).unwrap();
        for _ in 0..5 {
            w.step();
        }
        let p = w.player(me).unwrap();
        assert_eq!(count(&p.inventory, "mushrooms"), 2);
        assert_eq!(p.level("foraging"), 1);
        let npc = Command::CreateNpc {
            name: "Old Wren".into(),
            persona: "A forager who talks to birds.".into(),
        };
        assert!(w.apply(me, &npc).unwrap().contains("is here now"));
        assert_eq!(w.npcs_near(me).len(), 1);
        let id = w.npcs[0].id;
        w.npc_says(id, "The birds say hello.");
        let view = w.describe(me);
        assert!(
            view.contains("Old Wren (NPC, here)")
                && view.contains("Here: Damp Hollow")
                && view.contains("Old Wren says")
        );
        assert!(w
            .apply(
                me,
                &Command::MoveTo {
                    target: "Old Wren".into()
                }
            )
            .unwrap()
            .contains("already at Old Wren"));
    }

    #[test]
    fn targets_resolve_loosely() {
        let mut w = World::new(11);
        let me = w.join("Kyle");
        assert_eq!(
            w.place("the forest").map(|p| p.name.as_str()),
            Some("Old Forest")
        );
        assert_eq!(
            w.place("iron hill").map(|p| p.name.as_str()),
            Some("Iron Hill")
        );
        assert_eq!(
            w.place("river").map(|p| p.name.as_str()),
            Some("River Ford")
        );
        assert!(w.resolve_target(me, "north").is_some());
        assert!(w.resolve_target(me, "nowhere").is_none());
        assert_eq!(singular("fishes"), "fish");
        assert_eq!(singular("logs"), "log");
        assert_eq!(singular("moss"), "moss");
    }
}
