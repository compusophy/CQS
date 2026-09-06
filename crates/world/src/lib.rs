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

use gemini::Value;

pub const W: i32 = 48;
pub const H: i32 = 48;

/// Caps that keep one loud player from filling the world.
pub const MAX_NPCS_PER_PLAYER: usize = 5;
pub const MAX_PLACES_PER_PLAYER: usize = 8;
pub const MAX_RECIPES: usize = 12;
pub const MAX_QUEUE: usize = 24;
pub const NAME_MAX: usize = 24;
pub const WORD_MAX: usize = 20;
pub const TEXT_MAX: usize = 200;
pub const PERSONA_MAX: usize = 300;
/// A standing script is at most this many characters of Lua.
pub const SCRIPT_MAX: usize = 6000;
/// An idle script runs again only after this many ticks, so a script that
/// only talks or waits cannot fire every second.
pub const SCRIPT_REST: u64 = 5;
/// Saying the exact same line again within this many ticks is dropped.
pub const SAY_REPEAT_TICKS: u64 = 30;
/// An idle NPC's script runs again only after this many ticks.
pub const NPC_SCRIPT_REST: u64 = 10;

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
    /// What stands here: a banner on a spot, or a building with a footprint.
    pub form: Form,
    /// One word for its look ("stone", "dark", "red"), for the display.
    pub style: Option<String>,
    /// Materials still owed before the work can start.
    pub needs: Vec<(String, u32)>,
    /// Ticks of work done so far; `form.work()` of them finish it.
    pub work: u32,
}

impl Place {
    /// The footprint in tiles; (x, y) is its top-left corner.
    pub fn size(&self) -> (i32, i32) {
        self.form.size()
    }
    pub fn covers(&self, x: i32, y: i32) -> bool {
        let (w, h) = self.size();
        x >= self.x && y >= self.y && x < self.x + w && y < self.y + h
    }
    /// Chebyshev distance from a tile to the footprint; 0 inside it.
    pub fn dist(&self, x: i32, y: i32) -> i32 {
        let (w, h) = self.size();
        let dx = if x < self.x {
            self.x - x
        } else if x >= self.x + w {
            x - (self.x + w - 1)
        } else {
            0
        };
        let dy = if y < self.y {
            self.y - y
        } else if y >= self.y + h {
            y - (self.y + h - 1)
        } else {
            0
        };
        dx.max(dy)
    }
    /// Standing here counts as being at the place: beside a banner, or
    /// within two tiles of a building's walls (its door is one tile out, and
    /// arriving beside the door is arriving).
    pub fn near(&self, x: i32, y: i32) -> bool {
        self.dist(x, y) <= if self.form.blocks() { 2 } else { 1 }
    }
    pub fn built(&self) -> bool {
        self.needs.is_empty() && self.work >= self.form.work()
    }
    /// Solid: nobody walks through a wall.
    pub fn blocks(&self, x: i32, y: i32) -> bool {
        self.form.blocks() && self.covers(x, y)
    }
    /// Where to stand to be at it: the tile below the middle of the front.
    pub fn door(&self) -> (i32, i32) {
        let (w, h) = self.size();
        if self.form.blocks() {
            (self.x + w / 2, self.y + h)
        } else {
            (self.x, self.y)
        }
    }
    /// The middle of the footprint, for bearings.
    pub fn centre(&self) -> (i32, i32) {
        let (w, h) = self.size();
        (self.x + w / 2, self.y + h / 2)
    }
    /// "needs 40 stone, 10 wood".
    pub fn bill(&self) -> String {
        self.needs
            .iter()
            .map(|(r, n)| format!("{n} {r}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What a founded place is: a banner on a spot, free and instant, or a
/// building with a footprint, a bill of materials, and work to raise it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Banner,
    Hut,
    House,
    Hall,
    Tower,
    Spire,
    Forge,
    Mill,
    Shrine,
    Well,
}

impl Form {
    pub const ALL: [Form; 10] = [
        Form::Banner,
        Form::Hut,
        Form::House,
        Form::Hall,
        Form::Tower,
        Form::Spire,
        Form::Forge,
        Form::Mill,
        Form::Shrine,
        Form::Well,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Form::Banner => "banner",
            Form::Hut => "hut",
            Form::House => "house",
            Form::Hall => "hall",
            Form::Tower => "tower",
            Form::Spire => "spire",
            Form::Forge => "forge",
            Form::Mill => "mill",
            Form::Shrine => "shrine",
            Form::Well => "well",
        }
    }
    /// What people call things, mapped onto the forms the world can raise.
    pub fn parse(s: &str) -> Option<Form> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "" | "banner" | "spot" | "camp" | "clearing" | "flag" | "site" | "landing" => {
                Form::Banner
            }
            "hut" | "cabin" | "shack" | "cottage" | "hovel" | "lodge" | "hovel " => Form::Hut,
            "house" | "home" | "inn" | "tavern" | "shop" | "smokehouse" | "workshop" | "store" => {
                Form::House
            }
            "hall" | "manor" | "keep" | "castle" | "palace" | "barracks" | "guildhall"
            | "library" | "townhall" | "town hall" => Form::Hall,
            "tower" | "watchtower" | "lighthouse" | "belltower" | "bell tower" => Form::Tower,
            "spire" | "wizard tower" | "wizard's tower" | "wizards tower" | "mage tower"
            | "observatory" | "wizard's spire" => Form::Spire,
            "forge" | "smithy" | "blacksmith" | "foundry" | "furnace" | "smith" => Form::Forge,
            "mill" | "windmill" | "watermill" | "sawmill" | "lumbermill" | "lumber mill" => {
                Form::Mill
            }
            "shrine" | "temple" | "altar" | "chapel" | "church" | "monument" => Form::Shrine,
            "well" | "fountain" | "cistern" => Form::Well,
            _ => return None,
        })
    }
    /// Footprint in tiles.
    pub fn size(self) -> (i32, i32) {
        match self {
            Form::Banner | Form::Hut | Form::Shrine | Form::Well => (1, 1),
            Form::Hall => (3, 2),
            _ => (2, 2),
        }
    }
    pub fn blocks(self) -> bool {
        self != Form::Banner
    }
    /// The bill of materials, in resources the seeded world yields.
    pub fn cost(self) -> &'static [(&'static str, u32)] {
        match self {
            Form::Banner => &[],
            Form::Hut => &[("wood", 8)],
            Form::House => &[("wood", 20), ("stone", 6)],
            Form::Hall => &[("wood", 40), ("stone", 30)],
            Form::Tower => &[("stone", 40), ("wood", 10)],
            Form::Spire => &[("stone", 50), ("iron", 12), ("gold", 4)],
            Form::Forge => &[("stone", 20), ("iron", 8)],
            Form::Mill => &[("wood", 30), ("stone", 10)],
            Form::Shrine => &[("stone", 14), ("gold", 2)],
            Form::Well => &[("stone", 10)],
        }
    }
    /// Ticks of work once the materials are on site.
    pub fn work(self) -> u32 {
        match self {
            Form::Banner => 0,
            Form::Hut => 8,
            Form::House => 15,
            Form::Hall => 30,
            Form::Tower => 25,
            Form::Spire => 40,
            Form::Forge => 20,
            Form::Mill => 20,
            Form::Shrine => 12,
            Form::Well => 8,
        }
    }
    /// The cost table as one line, for whoever plans the gathering.
    pub fn costs_text() -> String {
        Form::ALL
            .iter()
            .filter(|f| **f != Form::Banner)
            .map(|f| {
                let (w, h) = f.size();
                format!(
                    "{} ({w}x{h}: {})",
                    f.name(),
                    f.cost()
                        .iter()
                        .map(|(r, n)| format!("{n} {r}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Npc {
    pub id: NpcId,
    pub name: String,
    pub persona: String,
    pub x: i32,
    pub y: i32,
    pub creator: PlayerId,
    /// What people have handed them.
    pub holds: Vec<(String, u32)>,
    /// What they are after, and what they give for it: a quest in one line.
    pub want: Option<Want>,
    /// Where they were made; a script's walk("home") goes back there.
    pub home: (i32, i32),
    pub task: Task,
    /// A standing script their maker gave them, run like a player's.
    pub script: Option<String>,
    pub memory: Value,
    pub script_tick: u64,
}

/// What an NPC wants and what it hands back — set by whoever made them.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub item: String,
    pub amount: u32,
    /// Handed over so far toward `amount`.
    pub given: u32,
    pub reward: Vec<(String, u32)>,
    /// A standing trade: met, it resets, rather than ending.
    pub repeat: bool,
    /// How its maker put the deal, for the voice.
    pub words: String,
}

impl Want {
    pub fn text(&self) -> String {
        let reward = if self.reward.is_empty() {
            "nothing but thanks".to_string()
        } else {
            goods_text(&self.reward)
        };
        format!(
            "{} {} for {reward} ({}/{}{})",
            self.amount,
            self.item,
            self.given,
            self.amount,
            if self.repeat { ", standing" } else { "" }
        )
    }
}

/// A thing somebody made: its name is a word in packs and wants; this is
/// what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub description: String,
    pub recipe: Vec<(String, u32)>,
    pub maker: PlayerId,
}

/// "2 fish, 1 gold".
pub fn goods_text(list: &[(String, u32)]) -> String {
    list.iter()
        .map(|(r, n)| format!("{n} {r}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// "2 fish and a gold coin" -> [("fish", 2), ("gold coin", 1)]. What people
/// write when they name goods.
pub fn goods(s: &str) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for part in s
        .replace(" and ", ",")
        .replace(" plus ", ",")
        .replace('+', ",")
        .split(',')
    {
        let mut words: Vec<&str> = part.split_whitespace().collect();
        let mut n = 1;
        if let Some(first) = words.first() {
            if let Some(k) = count_word(first) {
                n = k.max(1) as u32;
                words.remove(0);
            } else if matches!(*first, "a" | "an" | "some" | "the") {
                words.remove(0);
            }
        }
        let item = words.join(" ");
        if let Ok(item) = clean_item(&item) {
            add(&mut out, &item, n);
        }
    }
    out
}

/// The key in a pack that means `item`: the same word, or its plural.
fn held_key(list: &[(String, u32)], item: &str) -> Option<String> {
    let want = item.trim().to_lowercase();
    list.iter()
        .find(|(k, n)| *n > 0 && (*k == want || singular(k) == singular(&want)))
        .map(|(k, _)| k.clone())
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
    /// Raising a building whose materials are all on site.
    Build {
        site: String,
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
    /// A standing Lua script that decides what to do when idle. The world
    /// keeps it and its memory; a host runs it and reports back (`script_ran`).
    pub script: Option<String>,
    /// What the script remembers between runs, JSON-shaped.
    pub memory: Value,
    /// The tick the script last ran at, so one tick never runs it twice.
    pub script_tick: u64,
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

fn take(list: &mut Vec<(String, u32)>, key: &str, n: u32) {
    if let Some(slot) = list.iter_mut().find(|(k, _)| k == key) {
        slot.1 = slot.1.saturating_sub(n);
    }
    list.retain(|(_, n)| *n > 0);
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
    /// Found a named place where the character stands: a banner, or a
    /// building marked out with a bill of materials.
    FoundPlace {
        name: String,
        description: String,
        resource: Option<String>,
        skill: Option<String>,
        form: Form,
        style: Option<String>,
    },
    /// Carry materials to a site and work on it until it stands.
    Build { site: String },
    /// Tear down a place the character founded, site or building.
    Abandon { site: String },
    /// Hand something carried to a person within reach; no amount is all of it.
    Give {
        item: String,
        amount: Option<u32>,
        to: String,
    },
    /// Tell a character of one's own making what it wants and what it gives.
    SetWant {
        npc: String,
        item: String,
        amount: u32,
        reward: Vec<(String, u32)>,
        repeat: bool,
        words: String,
    },
    /// Make a thing from carried materials, at a built building.
    Craft {
        item: String,
        description: String,
        from: Vec<(String, u32)>,
    },
    /// Give a character of one's own making a standing script; empty clears.
    SetNpcScript { npc: String, source: String },
    /// Bring a character into the world where the player stands.
    CreateNpc { name: String, persona: String },
    /// Set the standing Lua script; an empty source clears it.
    SetScript { source: String },
}

impl Command {
    /// Starts, replaces, or ends a task — as opposed to happening at once.
    fn is_action(&self) -> bool {
        matches!(
            self,
            Command::MoveTo { .. }
                | Command::Gather { .. }
                | Command::Bank
                | Command::Build { .. }
                | Command::RunRecipe { .. }
                | Command::Stop
        )
    }
    /// Worth remembering as a step of a recipe.
    fn is_step(&self) -> bool {
        matches!(
            self,
            Command::MoveTo { .. }
                | Command::Gather { .. }
                | Command::Bank
                | Command::Build { .. }
                | Command::Say { .. }
                | Command::Give { .. }
                | Command::Craft { .. }
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
            Command::Build { site } => write!(f, "build {site}"),
            Command::Abandon { site } => write!(f, "abandon {site}"),
            Command::Give {
                item,
                amount: Some(n),
                to,
            } => write!(f, "give {n} {item} to {to}"),
            Command::Give {
                item,
                amount: None,
                to,
            } => write!(f, "give {item} to {to}"),
            Command::SetWant {
                npc, item, amount, ..
            } => {
                write!(f, "{npc} wants {amount} {item}")
            }
            Command::Craft { item, .. } => write!(f, "make {item}"),
            Command::SetNpcScript { npc, source } if source.trim().is_empty() => {
                write!(f, "clear {npc}'s script")
            }
            Command::SetNpcScript { npc, .. } => write!(f, "script for {npc}"),
            Command::CreateNpc { name, .. } => write!(f, "create {name}"),
            Command::SetScript { source } if source.trim().is_empty() => write!(f, "clear script"),
            Command::SetScript { .. } => write!(f, "set script"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub tick: u64,
    /// Who did it — a player or an NPC, by name.
    pub name: String,
    pub text: String,
    /// "note", "say", "voice" or "join": what a console should make of it.
    pub kind: &'static str,
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
    /// Everything anyone has made, by name.
    pub items: Vec<Item>,
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

/// "4", "four": how many tiles, when a direction comes with a distance.
fn count_word(w: &str) -> Option<i32> {
    if let Ok(n) = w.parse::<i32>() {
        return Some(n);
    }
    let words = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve",
    ];
    words.iter().position(|x| *x == w).map(|i| i as i32 + 1)
}

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

/// The name of a thing: lowercase, up to thirty characters of letters,
/// spaces, hyphens and apostrophes ("smoked fish", "fish-oil lantern").
fn clean_item(s: &str) -> Result<String, String> {
    let s = tidy(s, 30).to_lowercase();
    if s.chars().count() < 2
        || !s
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\''))
    {
        return Err(format!("'{s}' is not the name of a thing (letters)"));
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

/// "wren" names "Old Wren"; so does "old wren" and "Wren".
fn names_match(name: &str, q: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == q
        || n.split_whitespace().any(|w| w.len() >= 3 && q.contains(w))
        || q.split_whitespace().any(|w| w.len() >= 3 && n.contains(w))
}

enum Someone {
    Npc(NpcId),
    Player(PlayerId),
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
        let mut rx = 11 + rng.below(4);
        for y in 0..H {
            tiles[at(rx, y)] = Tile::Water;
            if rx + 1 < W && rng.below(3) == 0 {
                tiles[at(rx + 1, y)] = Tile::Water;
            }
            rx = (rx + rng.below(3) - 1).clamp(6, 17);
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
        let forest = (36 + rng.below(3), 31 + rng.below(3));
        let hill = (26 + rng.below(4), 8 + rng.below(3));
        let quarry = (41 + rng.below(3), 8 + rng.below(3));
        blob(&mut tiles, &mut rng, forest.0, forest.1, 7, Tile::Forest);
        blob(&mut tiles, &mut rng, hill.0, hill.1, 5, Tile::Hill);
        blob(&mut tiles, &mut rng, quarry.0, quarry.1, 3, Tile::Hill);
        // A lake in the south-east, a pine stand in the north-west, a wood
        // across the river in the south-west, a ridge in the far north-east:
        // scenery and room, not places — those are for players to found.
        let lake = (39 + rng.below(3), 42 + rng.below(2));
        let pines = (5 + rng.below(2), 6 + rng.below(3));
        let southwood = (5 + rng.below(3), 40 + rng.below(3));
        let ridge = (44 + rng.below(2), 20 + rng.below(4));
        blob(&mut tiles, &mut rng, lake.0, lake.1, 4, Tile::Water);
        blob(&mut tiles, &mut rng, pines.0, pines.1, 4, Tile::Forest);
        blob(
            &mut tiles,
            &mut rng,
            southwood.0,
            southwood.1,
            4,
            Tile::Forest,
        );
        blob(&mut tiles, &mut rng, ridge.0, ridge.1, 3, Tile::Hill);
        // Reeds where the lake drains south: a marsh, and the one seeded place
        // that is not a resource node of the first four.
        let marsh = (30 + rng.below(3), 43 + rng.below(2));
        blob(&mut tiles, &mut rng, marsh.0 + 3, marsh.1, 2, Tile::Water);
        let town = (24, 24);
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
        let creek = (7, 38 + rng.below(3));
        let seeded =
            |name: &str, (x, y): (i32, i32), res: Option<(&str, &str)>, desc: &str| Place {
                name: name.into(),
                x,
                y,
                resource: res.map(|(r, _)| r.into()),
                skill: res.map(|(_, s)| s.into()),
                description: desc.into(),
                founder: None,
                form: Form::Banner,
                style: None,
                needs: Vec::new(),
                work: 0,
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
            seeded(
                "Reed Marsh",
                marsh,
                Some(("reeds", "foraging")),
                "Standing water and whispering reeds where the lake drains. Things live in there.",
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
            items: Vec::new(),
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
        let (tx, ty) = (self.places[0].x, self.places[0].y);
        let (x, y) = self.free_spot_near(tx, ty, None);
        let name = name.into();
        self.players.push(Player {
            id,
            name: name.clone(),
            x,
            y,
            inventory: Vec::new(),
            bank: Vec::new(),
            xp: Vec::new(),
            task: Task::Idle,
            queue: VecDeque::new(),
            last_plan: Vec::new(),
            recipes: Vec::new(),
            looping: None,
            script: None,
            memory: Value::Null,
            script_tick: u64::MAX,
        });
        self.note_kind("join", &name, "arrived in Town");
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
        self.places.iter().find(|p| p.near(x, y))
    }
    /// A person within two tiles, by name: an NPC or another player.
    fn someone_near(&self, who: PlayerId, px: i32, py: i32, name: &str) -> Result<Someone, String> {
        let q = name.trim().to_ascii_lowercase();
        if q.is_empty() {
            return Err("give to whom?".into());
        }
        let reach = |x: i32, y: i32| (x - px).abs() <= 2 && (y - py).abs() <= 2;
        if let Some(n) = self.npcs.iter().find(|n| names_match(&n.name, &q)) {
            return if reach(n.x, n.y) {
                Ok(Someone::Npc(n.id))
            } else {
                Err(format!(
                    "{} is not within reach; walk to them first",
                    n.name
                ))
            };
        }
        if let Some(p) = self
            .players
            .iter()
            .find(|p| p.id != who && names_match(&p.name, &q))
        {
            return if reach(p.x, p.y) {
                Ok(Someone::Player(p.id))
            } else {
                Err(format!(
                    "{} is not within reach; walk to them first",
                    p.name
                ))
            };
        }
        Err(format!("there is nobody called '{name}' here"))
    }

    /// Water, or a building: nobody walks here.
    fn blocked(&self, x: i32, y: i32) -> bool {
        !self.tile(x, y).walkable() || self.places.iter().any(|p| p.blocks(x, y))
    }
    /// The tile to walk to for a place: its door, or the nearest open tile.
    fn approach(&self, p: &Place) -> (i32, i32) {
        let (dx, dy) = p.door();
        if dx >= 0 && dy >= 0 && dx < W && dy < H && !self.blocked(dx, dy) {
            (dx, dy)
        } else {
            self.free_spot_near(dx, dy, None)
        }
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
            self.note_kind("voice", &name, format!("says \"{}\"", tidy(text, TEXT_MAX)));
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

    /// The nearest spot to (x, y), within six tiles, where a footprint of
    /// `size` fits: open ground, a tile of gap from every other place, and
    /// nobody but the founder standing on it. Where a new place goes.
    fn place_spot(&self, x: i32, y: i32, size: (i32, i32), who: PlayerId) -> Option<(i32, i32)> {
        let (w, h) = size;
        // Banners block nothing, but they are 1x1 like a hut; the walk-around
        // check is cheap, so every footprint takes it.
        let blocks = true;
        let reach_now = self.reach(None);
        let mut best: Option<((i32, i32), i32)> = None;
        for dy in -6..=6i32 {
            for dx in -6..=6i32 {
                let (ax, ay) = (x + dx - w / 2, y + dy - h / 2);
                if ax < 0 || ay < 0 || ax + w > W || ay + h > H {
                    continue;
                }
                let d = dx.abs() + dy.abs();
                if best.is_some_and(|(_, bd)| d >= bd) {
                    continue;
                }
                let mut fits = true;
                'tiles: for ty in ay..ay + h {
                    for tx in ax..ax + w {
                        let t = self.tile(tx, ty);
                        // Not on water, the square, or a road: the ford is a
                        // road, and a house on the ford is an island.
                        if !t.walkable()
                            || t == Tile::Town
                            || t == Tile::Road
                            || self.places.iter().any(|p| p.dist(tx, ty) <= 1)
                            || self.occupied(tx, ty, Some(who))
                        {
                            fits = false;
                            break 'tiles;
                        }
                    }
                }
                if !fits {
                    continue;
                }
                // A wall must not cut the map: everything Town can reach now
                // must still be reachable around it.
                if blocks && self.reach(Some((ax, ay, w, h))) + (w * h) as usize != reach_now {
                    continue;
                }
                best = Some(((ax, ay), d));
            }
        }
        best.map(|(at, _)| at)
    }

    /// How many tiles Town can walk to, with an extra rect walled off.
    fn reach(&self, wall: Option<(i32, i32, i32, i32)>) -> usize {
        let inside = |x: i32, y: i32| {
            wall.is_some_and(|(wx, wy, ww, wh)| x >= wx && y >= wy && x < wx + ww && y < wy + wh)
        };
        let start = self.places[0].door();
        let idx = |(x, y): (i32, i32)| (y * W + x) as usize;
        let mut seen = vec![false; (W * H) as usize];
        let mut queue = VecDeque::new();
        seen[idx(start)] = true;
        queue.push_back(start);
        let mut n = 0;
        while let Some((cx, cy)) = queue.pop_front() {
            n += 1;
            for (_, (dx, dy)) in DIRS.iter().take(4) {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx < 0 || ny < 0 || nx >= W || ny >= H || seen[idx((nx, ny))] {
                    continue;
                }
                if self.blocked(nx, ny) || inside(nx, ny) {
                    continue;
                }
                seen[idx((nx, ny))] = true;
                queue.push_back((nx, ny));
            }
        }
        n
    }

    /// Players whose standing script should run now: idle, nothing queued,    /// Players whose standing script should run now: idle, nothing queued,
    /// no recipe on repeat, and not already run this tick.
    pub fn scripted_idle(&self) -> Vec<PlayerId> {
        self.players
            .iter()
            .filter(|p| {
                p.script.is_some()
                    && p.task == Task::Idle
                    && p.queue.is_empty()
                    && p.looping.is_none()
                    && (p.script_tick == u64::MAX || self.tick >= p.script_tick + SCRIPT_REST)
            })
            .map(|p| p.id)
            .collect()
    }

    /// A script ran, outside the world, on a host: here is what it decided,
    /// what it remembers, and anything it had to say about itself.
    pub fn script_ran(
        &mut self,
        who: PlayerId,
        cmds: Vec<Command>,
        memory: Value,
        note: &str,
    ) -> Result<String, String> {
        let name = self.name_of(who);
        let tick = self.tick;
        {
            let me = self.player_mut(who).ok_or("no such player")?;
            me.memory = memory;
            me.script_tick = tick;
        }
        if !note.is_empty() {
            self.note_kind("script", &name, tidy(note, TEXT_MAX));
        }
        if cmds.is_empty() {
            Ok(String::new())
        } else {
            self.plan(who, cmds)
        }
    }

    /// A line in the log from outside the world (a host, a script).
    pub fn log_event(&mut self, kind: &'static str, name: &str, text: impl Into<String>) {
        self.note_kind(kind, name, text);
    }

    fn note(&mut self, name: &str, text: impl Into<String>) {
        self.note_kind("note", name, text);
    }

    fn note_kind(&mut self, kind: &'static str, name: &str, text: impl Into<String>) {
        self.events.push(Event {
            tick: self.tick,
            name: name.to_string(),
            text: text.into(),
            kind,
        });
        if self.events.len() > 200 {
            self.events.drain(..100);
        }
    }

    fn occupied(&self, x: i32, y: i32, except: Option<PlayerId>) -> bool {
        self.players
            .iter()
            .any(|p| Some(p.id) != except && p.pos() == (x, y))
            || self.npcs.iter().any(|n| (n.x, n.y) == (x, y))
    }

    /// The nearest walkable tile to (x, y) that nobody stands on, searching
    /// outward ring by ring. `except` is the one asking, who may keep their
    /// own tile. Nobody ever stands on anybody.
    pub fn free_spot_near(&self, x: i32, y: i32, except: Option<PlayerId>) -> (i32, i32) {
        for r in 0i32..=8 {
            let mut best: Option<((i32, i32), i32)> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) != r {
                        continue;
                    }
                    let (tx, ty) = (x + dx, y + dy);
                    if tx < 0 || ty < 0 || tx >= W || ty >= H {
                        continue;
                    }
                    if self.blocked(tx, ty) || self.occupied(tx, ty, except) {
                        continue;
                    }
                    let d = dx * dx + dy * dy;
                    if best.map_or(true, |(_, bd)| d < bd) {
                        best = Some(((tx, ty), d));
                    }
                }
            }
            if let Some((spot, _)) = best {
                return spot;
            }
        }
        (x, y)
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
                if dest == (px, py)
                    || (self.occupied(dest.0, dest.1, Some(who)) && near(dest.0, dest.1, px, py))
                {
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
                let here = src.near(px, py);
                let to = self.approach(&src);
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
                        to,
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
                if town.near(px, py) {
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
                    let to = self.approach(&town);
                    self.player_mut(who).unwrap().task = Task::Walk {
                        to,
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
                let line = format!("says \"{text}\"");
                let tick = self.tick;
                if self.events.iter().rev().any(|e| {
                    e.kind == "say"
                        && e.name == name
                        && e.text == line
                        && e.tick + SAY_REPEAT_TICKS > tick
                }) {
                    return Ok(format!("{name} already said that."));
                }
                self.note_kind("say", &name, line);
                // Within earshot (two tiles), the one addressed by name answers;
                // otherwise whoever is nearest.
                let lower = text.to_ascii_lowercase();
                let mut heard: Vec<&Npc> = self
                    .npcs
                    .iter()
                    .filter(|n| (n.x - px).abs() <= 2 && (n.y - py).abs() <= 2)
                    .collect();
                heard.sort_by_key(|n| (n.x - px).abs() + (n.y - py).abs());
                let named = heard.iter().find(|n| {
                    n.name
                        .split_whitespace()
                        .any(|w| w.len() >= 3 && lower.contains(&w.to_ascii_lowercase()))
                });
                if let Some(n) = named.or_else(|| heard.first()) {
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
                form,
                style,
            } => {
                let pname = clean_name(pname)?;
                if self
                    .places
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(&pname))
                {
                    return Err(format!("there is already a place called {pname}"));
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
                let form = *form;
                // A place needs room: the nearest spot its footprint fits with
                // a gap from everything else. The world finds it rather than
                // sending the founder off to look for one.
                let Some((sx, sy)) = self.place_spot(px, py, form.size(), who) else {
                    return Err(format!(
                        "no room for a {} here; somewhere more open",
                        form.name()
                    ));
                };
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
                let style = style
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .map(clean_word)
                    .transpose()?;
                let description = tidy(description, TEXT_MAX);
                let needs: Vec<(String, u32)> = form
                    .cost()
                    .iter()
                    .map(|(r, n)| (r.to_string(), *n))
                    .collect();
                let place = Place {
                    name: pname.clone(),
                    x: sx,
                    y: sy,
                    resource: resource.clone(),
                    skill,
                    description,
                    founder: Some(who),
                    form,
                    style,
                    needs,
                    work: 0,
                };
                let d = place.dist(px, py);
                let (cx, cy) = place.centre();
                let bill = place.bill();
                self.places.push(place.clone());
                // Nobody stands inside a wall: whoever was on the footprint
                // steps off it.
                let inside: Vec<PlayerId> = self
                    .players
                    .iter()
                    .filter(|p| place.blocks(p.x, p.y))
                    .map(|p| p.id)
                    .collect();
                for id in inside {
                    let (fx, fy) = self.free_spot_near(cx, cy, Some(id));
                    let p = self.player_mut(id).unwrap();
                    p.x = fx;
                    p.y = fy;
                }
                let where_ = if d == 0 {
                    String::new()
                } else {
                    format!(", {d} {}", compass(px, py, cx, cy))
                };
                if form == Form::Banner {
                    self.note(&name, format!("founded {pname}{where_}"));
                    Ok(match resource {
                        Some(r) => format!("{name} founds {pname}{where_}. It yields {r}."),
                        None => format!("{name} founds {pname}{where_}."),
                    })
                } else {
                    self.note_kind(
                        "build",
                        &name,
                        format!(
                            "marked out {pname}, a {}{where_}; it needs {bill}",
                            form.name()
                        ),
                    );
                    Ok(format!(
                        "{name} marks out {pname}, a {}{where_}. It needs {bill}: carry the materials there and build.",
                        form.name()
                    ))
                }
            }
            Command::Build { site } => {
                let site = self.place(site).cloned().ok_or_else(|| {
                    let sites: Vec<&str> = self
                        .places
                        .iter()
                        .filter(|p| !p.built())
                        .map(|p| p.name.as_str())
                        .collect();
                    format!(
                        "there is no site called '{site}'. Unfinished: {}",
                        if sites.is_empty() {
                            "none".to_string()
                        } else {
                            sites.join(", ")
                        }
                    )
                })?;
                if site.built() {
                    return Err(format!("{} is already built", site.name));
                }
                if !site.near(px, py) {
                    let to = self.approach(&site);
                    self.player_mut(who).unwrap().task = Task::Walk {
                        to,
                        then: Some(Box::new(Command::Build {
                            site: site.name.clone(),
                        })),
                    };
                    self.note(&name, format!("set out for {} to build", site.name));
                    return Ok(format!("{name} heads for {} to build.", site.name));
                }
                // Hand over whatever is carried and owed.
                let idx = self
                    .places
                    .iter()
                    .position(|p| p.name == site.name)
                    .unwrap();
                let carried = self.player(who).unwrap().inventory.clone();
                let delivered: Vec<(String, u32)> = site
                    .needs
                    .iter()
                    .filter_map(|(r, owed)| {
                        let give = count(&carried, r).min(*owed);
                        (give > 0).then(|| (r.clone(), give))
                    })
                    .collect();
                if !delivered.is_empty() {
                    let me = self.player_mut(who).unwrap();
                    for (r, n) in &delivered {
                        take(&mut me.inventory, r, *n);
                    }
                    let pl = &mut self.places[idx];
                    for (r, n) in &delivered {
                        if let Some(slot) = pl.needs.iter_mut().find(|(k, _)| k == r) {
                            slot.1 -= n;
                        }
                    }
                    pl.needs.retain(|(_, n)| *n > 0);
                    let list: Vec<String> =
                        delivered.iter().map(|(r, n)| format!("{n} {r}")).collect();
                    self.note_kind(
                        "build",
                        &name,
                        format!("delivered {} to {}", list.join(", "), site.name),
                    );
                }
                let pl = self.places[idx].clone();
                if !pl.needs.is_empty() {
                    let bill = pl.bill();
                    if delivered.is_empty() {
                        return Err(format!(
                            "{} still needs {bill}; gather it and carry it here",
                            pl.name
                        ));
                    }
                    return Ok(format!(
                        "{name} delivers to {}. It still needs {bill}.",
                        pl.name
                    ));
                }
                self.player_mut(who).unwrap().task = Task::Build {
                    site: pl.name.clone(),
                };
                self.note_kind("build", &name, format!("began building {}", pl.name));
                Ok(format!(
                    "{name} begins building {} ({} ticks of work).",
                    pl.name,
                    pl.form.work().saturating_sub(pl.work)
                ))
            }
            Command::Abandon { site } => {
                let target = self
                    .place(site)
                    .map(|p| p.name.clone())
                    .ok_or_else(|| format!("there is no place called '{site}'"))?;
                let idx = self.places.iter().position(|p| p.name == target).unwrap();
                if self.places[idx].founder != Some(who) {
                    return Err(format!("{target} is not {name}'s to tear down"));
                }
                let pl = self.places.remove(idx);
                self.note_kind(
                    "build",
                    &name,
                    format!("tore down {}, a {}", pl.name, pl.form.name()),
                );
                Ok(format!("{name} tears down {}.", pl.name))
            }
            Command::Give { item, amount, to } => {
                let item = clean_item(item)?;
                let inv = self.player(who).unwrap().inventory.clone();
                let key = held_key(&inv, &item)
                    .ok_or_else(|| format!("{name} is not carrying any {item}"))?;
                let have = count(&inv, &key);
                let n = amount.unwrap_or(have).min(have);
                if n == 0 {
                    return Err(format!("{name} has no {key} to give"));
                }
                let someone = self.someone_near(who, px, py, to)?;
                take(&mut self.player_mut(who).unwrap().inventory, &key, n);
                match someone {
                    Someone::Player(pid) => {
                        let other = self.player_mut(pid).unwrap();
                        add(&mut other.inventory, &key, n);
                        let oname = other.name.clone();
                        self.note_kind("give", &name, format!("gave {n} {key} to {oname}"));
                        Ok(format!("{name} gives {n} {key} to {oname}."))
                    }
                    Someone::Npc(id) => {
                        let tick = self.tick;
                        let npc = self.npcs.iter_mut().find(|x| x.id == id).unwrap();
                        add(&mut npc.holds, &key, n);
                        let nname = npc.name.clone();
                        // The want, if this is what they were after.
                        let mut paid: Vec<(String, u32)> = Vec::new();
                        let mut met = false;
                        if let Some(w) = &mut npc.want {
                            if singular(&w.item) == singular(&key) {
                                w.given += n;
                                if w.given >= w.amount {
                                    met = true;
                                    paid = w.reward.clone();
                                    if w.repeat {
                                        w.given -= w.amount;
                                    }
                                }
                            }
                        }
                        if met && !npc.want.as_ref().is_some_and(|w| w.repeat) {
                            npc.want = None;
                        }
                        self.speeches.push(Speech {
                            tick,
                            speaker: who,
                            listener: id,
                            text: format!("*hands over {n} {key}"),
                        });
                        self.note_kind("give", &name, format!("gave {n} {key} to {nname}"));
                        if met {
                            let me = self.player_mut(who).unwrap();
                            for (r, k) in &paid {
                                add(&mut me.inventory, r, *k);
                            }
                            let what = if paid.is_empty() {
                                "thanks".to_string()
                            } else {
                                goods_text(&paid)
                            };
                            self.note_kind(
                                "give",
                                &nname,
                                format!("gives {name} {what} for the {key}"),
                            );
                            return Ok(format!(
                                "{name} gives {n} {key} to {nname}. {nname} gives {name} {what} for the {key}."
                            ));
                        }
                        Ok(format!("{name} gives {n} {key} to {nname}."))
                    }
                }
            }
            Command::SetWant {
                npc,
                item,
                amount,
                reward,
                repeat,
                words,
            } => {
                let q = npc.trim().to_ascii_lowercase();
                let id = self
                    .npcs
                    .iter()
                    .find(|n| names_match(&n.name, &q))
                    .map(|n| n.id)
                    .ok_or_else(|| format!("there is nobody called '{npc}'"))?;
                let item = clean_item(item)?;
                let amount = (*amount).max(1);
                let reward: Vec<(String, u32)> = reward
                    .iter()
                    .map(|(r, k)| Ok((clean_item(r)?, (*k).max(1))))
                    .collect::<Result<_, String>>()?;
                let n = self.npcs.iter_mut().find(|n| n.id == id).unwrap();
                if n.creator != who {
                    return Err(format!("{} is not {name}'s to direct", n.name));
                }
                n.want = Some(Want {
                    item,
                    amount,
                    given: 0,
                    reward,
                    repeat: *repeat,
                    words: tidy(words, TEXT_MAX),
                });
                let nname = n.name.clone();
                let text = n.want.as_ref().unwrap().text();
                self.note(&nname, format!("now wants {text}"));
                Ok(format!("{nname} now wants {text}."))
            }
            Command::Craft {
                item,
                description,
                from,
            } => {
                let item = clean_item(item)?;
                if from.is_empty() {
                    return Err(format!("say what {item} is made from"));
                }
                let shop = self
                    .places
                    .iter()
                    .find(|pl| pl.form.blocks() && pl.built() && pl.near(px, py))
                    .cloned()
                    .ok_or(
                        "making things takes a workshop: stand at a built building (a forge, a mill, a hut...)",
                    )?;
                let inv = self.player(who).unwrap().inventory.clone();
                let mut used: Vec<(String, u32)> = Vec::new();
                for (m, k) in from {
                    let k = (*k).max(1);
                    let key = held_key(&inv, m)
                        .filter(|key| count(&inv, key) >= k)
                        .ok_or_else(|| {
                            format!(
                                "{name} needs {k} {m} to make {item}; carrying: {}",
                                if inv.is_empty() {
                                    "nothing".to_string()
                                } else {
                                    goods_text(&inv)
                                }
                            )
                        })?;
                    add(&mut used, &key, k);
                }
                let total: u32 = used.iter().map(|(_, k)| *k).sum();
                {
                    let me = self.player_mut(who).unwrap();
                    for (r, k) in &used {
                        take(&mut me.inventory, r, *k);
                    }
                    add(&mut me.inventory, &item, 1);
                    add(&mut me.xp, "crafting", 10 * total);
                }
                let description = tidy(description, TEXT_MAX);
                if !self.items.iter().any(|i| i.name == item) {
                    self.items.push(Item {
                        name: item.clone(),
                        description: description.clone(),
                        recipe: used.clone(),
                        maker: who,
                    });
                }
                let from_text = goods_text(&used);
                self.note_kind(
                    "craft",
                    &name,
                    format!("made a {item} at {} from {from_text}", shop.name),
                );
                Ok(format!(
                    "{name} makes a {item} at {} from {from_text}.",
                    shop.name
                ))
            }
            Command::SetNpcScript { npc, source } => {
                let q = npc.trim().to_ascii_lowercase();
                let id = self
                    .npcs
                    .iter()
                    .find(|n| names_match(&n.name, &q))
                    .map(|n| n.id)
                    .ok_or_else(|| format!("there is nobody called '{npc}'"))?;
                let source = source.trim();
                if source.chars().count() > SCRIPT_MAX {
                    return Err(format!("a script is at most {SCRIPT_MAX} characters"));
                }
                let n = self.npcs.iter_mut().find(|n| n.id == id).unwrap();
                if n.creator != who {
                    return Err(format!("{} is not {name}'s to direct", n.name));
                }
                let nname = n.name.clone();
                n.memory = Value::Null;
                n.script_tick = u64::MAX;
                if source.is_empty() {
                    n.script = None;
                    self.note_kind("script", &nname, "has no script now");
                    return Ok(format!("{nname} has no script now."));
                }
                let lines = source.lines().count();
                n.script = Some(source.to_string());
                self.note_kind(
                    "script",
                    &nname,
                    format!("was given a script of {lines} lines"),
                );
                Ok(format!("{nname} has a script now ({lines} lines)."))
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
                let (nx, ny) = self.free_spot_near(px, py, None);
                self.npcs.push(Npc {
                    id,
                    name: nname.clone(),
                    persona,
                    x: nx,
                    y: ny,
                    creator: who,
                    holds: Vec::new(),
                    want: None,
                    home: (nx, ny),
                    task: Task::Idle,
                    script: None,
                    memory: Value::Null,
                    script_tick: u64::MAX,
                });
                self.note(&name, format!("brought {nname} into the world"));
                Ok(format!("{nname} is here now, beside {name}."))
            }
            Command::SetScript { source } => {
                let source = source.trim();
                if source.chars().count() > SCRIPT_MAX {
                    return Err(format!("a script is at most {SCRIPT_MAX} characters"));
                }
                let me = self.player_mut(who).unwrap();
                me.memory = Value::Null;
                if source.is_empty() {
                    me.script = None;
                    self.note_kind("script", &name, "cleared the script");
                    return Ok(format!("{name} clears the script."));
                }
                let lines = source.lines().count();
                me.script = Some(source.to_string());
                me.script_tick = u64::MAX; // never run yet
                self.note_kind("script", &name, format!("set a script of {lines} lines"));
                Ok(format!("{name} sets a script ({lines} lines)."))
            }
        }
    }

    /// A place, a person, or a compass direction (five tiles that way, walked
    /// back onto land). Returns the destination and how to name it.
    fn resolve_target(&self, who: PlayerId, target: &str) -> Option<((i32, i32), String)> {
        let from = self.player(who).map(|p| p.pos())?;
        self.resolve_from(from, Some(who), None, target)
    }

    /// The same from anywhere, for NPCs, who are nobody's player.
    fn resolve_from(
        &self,
        from: (i32, i32),
        who: Option<PlayerId>,
        me: Option<NpcId>,
        target: &str,
    ) -> Option<((i32, i32), String)> {
        // People by exact name first, so "Old Wren" is not "Old Forest".
        let q = target.trim().to_ascii_lowercase();
        if let Some(n) = self
            .npcs
            .iter()
            .find(|n| Some(n.id) != me && n.name.to_ascii_lowercase() == q)
        {
            return Some(((n.x, n.y), n.name.clone()));
        }
        if let Some(p) = self
            .players
            .iter()
            .find(|p| Some(p.id) != who && p.name.to_ascii_lowercase() == q)
        {
            return Some(((p.x, p.y), p.name.clone()));
        }
        if let Some(p) = self.place(target) {
            return Some((self.approach(p), p.name.clone()));
        }
        let t = q
            .trim_start_matches("go ")
            .trim_start_matches("to ")
            .trim_start_matches("the ")
            .trim();
        // "4 tiles south", "south 4", "four south": a count and a direction.
        let mut count: Option<i32> = None;
        let mut words: Vec<&str> = Vec::new();
        for word in t.split_whitespace() {
            if let Some(n) = count_word(word) {
                count = Some(n);
            } else if !matches!(
                word,
                "tiles" | "tile" | "steps" | "step" | "paces" | "squares" | "a" | "few"
            ) {
                words.push(word);
            }
        }
        let t = words.join(" ");
        let t = match t.as_str() {
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
        let most = count.unwrap_or(5).clamp(1, 12);
        for step in (1..=most).rev() {
            let x = (from.0 + dx * step).clamp(0, W - 1);
            let y = (from.1 + dy * step).clamp(0, H - 1);
            if !self.blocked(x, y) {
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
                        // Nobody stands on anybody: settle onto the nearest free
                        // tile that still counts as being there.
                        if self.occupied(p.x, p.y, Some(id)) {
                            let (sx, sy) = self.free_spot_near(to.0, to.1, Some(id));
                            if then.is_none() || near(sx, sy, to.0, to.1) {
                                let me = self.player_mut(id).unwrap();
                                me.x = sx;
                                me.y = sy;
                            }
                        }
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
                            pl.near(p.x, p.y) && pl.resource.as_deref() == Some(resource.as_str())
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
                Task::Build { site } => {
                    let found = self.places.iter().position(|pl| pl.name == site);
                    match found {
                        Some(idx)
                            if self.places[idx].near(p.x, p.y)
                                && self.places[idx].needs.is_empty() =>
                        {
                            self.places[idx].work += 1;
                            let me = self.player_mut(id).unwrap();
                            let before = me.level("building");
                            add(&mut me.xp, "building", 10);
                            let after = me.level("building");
                            if after > before {
                                self.note(&p.name, format!("reached building level {after}"));
                            }
                            if self.places[idx].built() {
                                let pl = self.places[idx].clone();
                                self.player_mut(id).unwrap().task = Task::Idle;
                                self.note_kind(
                                    "build",
                                    &p.name,
                                    format!("raised {}, a {}", pl.name, pl.form.name()),
                                );
                            }
                        }
                        _ => {
                            let me = self.player_mut(id).unwrap();
                            me.task = Task::Idle;
                            me.queue.clear();
                            me.looping = None;
                            self.note(&p.name, format!("stopped building {site}"));
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
        // NPCs on their way somewhere.
        let nids: Vec<NpcId> = self.npcs.iter().map(|n| n.id).collect();
        for nid in nids {
            let n = self.npcs.iter().find(|n| n.id == nid).unwrap().clone();
            let Task::Walk { to, .. } = n.task else {
                continue;
            };
            let arrived =
                (n.x, n.y) == to || (near(n.x, n.y, to.0, to.1) && self.occupied(to.0, to.1, None));
            let next = if arrived {
                None
            } else {
                self.path_step((n.x, n.y), to)
            };
            let npc = self.npcs.iter_mut().find(|n| n.id == nid).unwrap();
            match next {
                Some((x, y)) => {
                    npc.x = x;
                    npc.y = y;
                }
                None => npc.task = Task::Idle,
            }
        }
        self.hail();
    }

    /// NPCs whose script should run now: idle and rested.
    pub fn npc_scripted_idle(&self) -> Vec<NpcId> {
        self.npcs
            .iter()
            .filter(|n| {
                n.script.is_some()
                    && n.task == Task::Idle
                    && (n.script_tick == u64::MAX || self.tick >= n.script_tick + NPC_SCRIPT_REST)
            })
            .map(|n| n.id)
            .collect()
    }

    /// An NPC's script ran on a host: what it decided, what it remembers.
    pub fn npc_ran(
        &mut self,
        id: NpcId,
        cmds: Vec<Command>,
        memory: Value,
        note: &str,
    ) -> Result<String, String> {
        let tick = self.tick;
        let name = {
            let n = self
                .npcs
                .iter_mut()
                .find(|n| n.id == id)
                .ok_or("no such npc")?;
            n.memory = memory;
            n.script_tick = tick;
            n.name.clone()
        };
        if !note.is_empty() {
            self.note_kind("script", &name, tidy(note, TEXT_MAX));
        }
        let mut acks = Vec::new();
        for c in &cmds {
            match self.apply_npc(id, c) {
                Ok(a) if !a.is_empty() => acks.push(a),
                Ok(_) => {}
                Err(e) => acks.push(format!("x {e}")),
            }
        }
        Ok(acks.join("\n"))
    }

    /// What an NPC can do on its own: walk, speak, hand something over.
    fn apply_npc(&mut self, id: NpcId, cmd: &Command) -> Result<String, String> {
        let n = self
            .npcs
            .iter()
            .find(|n| n.id == id)
            .ok_or("no such npc")?
            .clone();
        match cmd {
            Command::MoveTo { target } => {
                let (to, label) = if target.trim().eq_ignore_ascii_case("home") {
                    (n.home, "home".to_string())
                } else {
                    self.resolve_from((n.x, n.y), None, Some(id), target)
                        .ok_or_else(|| format!("{} has nowhere called '{target}' to go", n.name))?
                };
                if to == (n.x, n.y) {
                    return Ok(String::new());
                }
                let npc = self.npcs.iter_mut().find(|n| n.id == id).unwrap();
                npc.task = Task::Walk { to, then: None };
                Ok(format!("{} sets off for {label}.", n.name))
            }
            Command::Stop => {
                let npc = self.npcs.iter_mut().find(|n| n.id == id).unwrap();
                npc.task = Task::Idle;
                Ok(String::new())
            }
            Command::Look => Ok(String::new()),
            Command::Say { text } => {
                let text = tidy(text, TEXT_MAX);
                if text.is_empty() {
                    return Err("nothing to say".into());
                }
                let line = format!("says \"{text}\"");
                let tick = self.tick;
                if self.events.iter().rev().any(|e| {
                    e.kind == "voice"
                        && e.name == n.name
                        && e.text == line
                        && e.tick + SAY_REPEAT_TICKS > tick
                }) {
                    return Ok(String::new());
                }
                self.note_kind("voice", &n.name, line);
                Ok(format!("{} says \"{text}\"", n.name))
            }
            Command::Give { item, amount, to } => {
                let item = clean_item(item)?;
                let key = held_key(&n.holds, &item)
                    .ok_or_else(|| format!("{} holds no {item}", n.name))?;
                let have = count(&n.holds, &key);
                let k = amount.unwrap_or(have).min(have);
                if k == 0 {
                    return Err(format!("{} has no {key} to give", n.name));
                }
                let q = to.trim().to_ascii_lowercase();
                let pid = self
                    .players
                    .iter()
                    .find(|p| {
                        names_match(&p.name, &q) && (p.x - n.x).abs() <= 2 && (p.y - n.y).abs() <= 2
                    })
                    .map(|p| p.id)
                    .ok_or_else(|| format!("{to} is not within {}'s reach", n.name))?;
                take(
                    &mut self.npcs.iter_mut().find(|n| n.id == id).unwrap().holds,
                    &key,
                    k,
                );
                let p = self.player_mut(pid).unwrap();
                add(&mut p.inventory, &key, k);
                let pname = p.name.clone();
                self.note_kind("give", &n.name, format!("gave {k} {key} to {pname}"));
                Ok(format!("{} gives {k} {key} to {pname}.", n.name))
            }
            other => Err(format!("{} cannot {other}", n.name)),
        }
    }

    /// An NPC with a want calls out to whoever comes by carrying it: a cue
    /// for the voice, at most once in forty ticks, never while one is
    /// already waiting to be answered.
    fn hail(&mut self) {
        let tick = self.tick;
        let mut cues = Vec::new();
        for n in &self.npcs {
            let Some(w) = &n.want else { continue };
            if self.speeches.iter().any(|s| s.listener == n.id) {
                continue;
            }
            let spoke_lately = self
                .events
                .iter()
                .rev()
                .take(60)
                .any(|e| e.kind == "voice" && e.name == n.name && e.tick + 40 > tick);
            if spoke_lately {
                continue;
            }
            let passer = self.players.iter().find(|p| {
                (p.x - n.x).abs() <= 2
                    && (p.y - n.y).abs() <= 2
                    && held_key(&p.inventory, &w.item).is_some()
            });
            if let Some(p) = passer {
                let key = held_key(&p.inventory, &w.item).unwrap();
                cues.push(Speech {
                    tick,
                    speaker: p.id,
                    listener: n.id,
                    text: format!("*comes near, carrying {} {key}", count(&p.inventory, &key)),
                });
            }
        }
        self.speeches.extend(cues);
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
                if self.blocked(n.0, n.1) || prev[idx(n)] != u32::MAX {
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
            Task::Build { site } => format!("building {site}"),
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
                "Here: {} — \"{}\"{founder}{}\n",
                pl.name,
                pl.description,
                standing(pl)
            ));
        }
        s.push_str("Places: ");
        let mut first = true;
        for pl in &self.places {
            if !first {
                s.push_str(", ");
            }
            first = false;
            let d = pl.dist(p.x, p.y);
            let (cx, cy) = pl.centre();
            let where_ = if d == 0 {
                "here".to_string()
            } else {
                format!("{d} {}", compass(p.x, p.y, cx, cy))
            };
            let what = standing(pl);
            match (&pl.resource, &pl.skill) {
                (Some(r), Some(sk)) => {
                    s.push_str(&format!("{} ({where_}, {r}/{sk}{what})", pl.name))
                }
                (Some(r), None) => s.push_str(&format!("{} ({where_}, {r}{what})", pl.name)),
                _ => s.push_str(&format!("{} ({where_}{what})", pl.name)),
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
                    Task::Build { site } => format!("building {site}"),
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
            let mut about = String::new();
            if !n.holds.is_empty() {
                about.push_str(&format!("; holds {}", goods_text(&n.holds)));
            }
            if let Some(w) = &n.want {
                about.push_str(&format!("; wants {}", w.text()));
            }
            if matches!(n.task, Task::Walk { .. }) {
                about.push_str("; walking");
            }
            if d <= 1 {
                people.push(format!("{} (NPC, here{about})", n.name));
            } else {
                people.push(format!(
                    "{} (NPC, {d} {}{about})",
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
            .map(|(r, n)| match self.items.iter().find(|i| i.name == *r) {
                Some(i) => format!("{n} {r} ({})", tidy(&i.description, 60)),
                None => format!("{n} {r}"),
            })
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

/// What a place is, after its name: nothing for a banner, else its form
/// and, until it stands, what it still wants.
fn standing(pl: &Place) -> String {
    match pl.form {
        Form::Banner => String::new(),
        f if pl.built() => format!(", {}", f.name()),
        f if !pl.needs.is_empty() => format!(", {} site: needs {}", f.name(), pl.bill()),
        f => format!(", {} under construction {}/{}", f.name(), pl.work, f.work()),
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
    use gemini::obj;

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
        let me = w.join("Ada");
        for pl in w.places.clone() {
            w.apply(
                me,
                &Command::MoveTo {
                    target: pl.name.clone(),
                },
            )
            .unwrap();
            for _ in 0..140 {
                w.step();
            }
            let at = w.player(me).unwrap().pos();
            assert!(near(at.0, at.1, pl.x, pl.y), "did not reach {}", pl.name);
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
        let me = w.join("Ada");
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
        for _ in 0..14 {
            w.step();
        }
        w.apply(me, &Command::Say { text: "far".into() }).unwrap();
        assert!(w.take_speeches().is_empty());
    }

    #[test]
    fn players_found_places_with_any_resource_and_create_npcs() {
        let mut w = World::new(5);
        let me = w.join("Ada");
        assert!(w
            .apply(me, &gather("mushrooms", None))
            .unwrap()
            .starts_with("x nothing in this world yields"));
        let found = Command::FoundPlace {
            name: "Damp Hollow".into(),
            description: "Mushrooms under every log.".into(),
            resource: Some("Mushrooms".into()),
            skill: Some("foraging".into()),
            form: Form::Banner,
            style: None,
        };
        // Town is right here; the world puts the hollow a few tiles off.
        let r = w.apply(me, &found).unwrap();
        assert!(
            r.starts_with("Ada founds Damp Hollow, ") && r.contains("It yields mushrooms"),
            "{r}"
        );
        w.apply(
            me,
            &Command::MoveTo {
                target: "Damp Hollow".into(),
            },
        )
        .unwrap();
        for _ in 0..8 {
            w.step();
        }
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
        let me = w.join("Ada");
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
        let (x, y) = w.player(me).unwrap().pos();
        assert_eq!(w.resolve_target(me, "2 tiles north").unwrap().0, (x, y - 2));
        assert_eq!(w.resolve_target(me, "east 3").unwrap().0, (x + 3, y));
        assert_eq!(w.resolve_target(me, "go four south").unwrap().0, (x, y + 4));
        assert_eq!(
            w.resolve_target(me, "one step west").unwrap().1,
            "1 tiles west"
        );
        assert_eq!(singular("fishes"), "fish");
        assert_eq!(singular("logs"), "log");
        assert_eq!(singular("moss"), "moss");
    }

    #[test]
    fn repeating_a_line_is_dropped_and_scripts_rest_between_runs() {
        let mut w = World::new(5);
        let me = w.join("Ada");
        let said = |w: &World| w.events.iter().filter(|e| e.kind == "say").count();
        w.apply(
            me,
            &Command::Say {
                text: "hello".into(),
            },
        )
        .unwrap();
        assert_eq!(said(&w), 1);
        let r = w
            .apply(
                me,
                &Command::Say {
                    text: "hello".into(),
                },
            )
            .unwrap();
        assert!(r.contains("already said"), "{r}");
        assert_eq!(said(&w), 1);
        w.apply(
            me,
            &Command::Say {
                text: "hello again".into(),
            },
        )
        .unwrap();
        assert_eq!(said(&w), 2);
        for _ in 0..SAY_REPEAT_TICKS {
            w.step();
        }
        w.apply(
            me,
            &Command::Say {
                text: "hello".into(),
            },
        )
        .unwrap();
        assert_eq!(said(&w), 3, "after a while the same line is fine again");

        w.apply(
            me,
            &Command::SetScript {
                source: "say('hi')".into(),
            },
        )
        .unwrap();
        assert_eq!(w.scripted_idle(), vec![me], "a fresh script runs at once");
        w.script_ran(me, vec![], Value::Null, "").unwrap();
        assert!(w.scripted_idle().is_empty(), "and then it rests");
        for _ in 0..SCRIPT_REST {
            w.step();
        }
        assert_eq!(w.scripted_idle(), vec![me]);
    }

    #[test]
    fn a_building_is_marked_out_supplied_and_raised_and_nobody_walks_through_it() {
        let mut w = World::new(5);
        let me = w.join("Ada");
        let tower = Command::FoundPlace {
            name: "Grey Spire".into(),
            description: "A wizard's tower.".into(),
            resource: None,
            skill: None,
            form: Form::Spire,
            style: Some("dark".into()),
        };
        let r = w.apply(me, &tower).unwrap();
        assert!(
            r.contains("marks out Grey Spire, a spire") && r.contains("It needs 50 stone"),
            "{r}"
        );
        let site = w.place("Grey Spire").unwrap().clone();
        assert!(!site.built());
        assert_eq!(site.size(), (2, 2));
        let p = w.player(me).unwrap().clone();
        assert!(
            !site.blocks(p.x, p.y),
            "the founder stepped off the footprint"
        );
        assert!(w.blocked(site.x, site.y) && w.blocked(site.x + 1, site.y + 1));
        assert!(w
            .describe(me)
            .contains("spire site: needs 50 stone, 12 iron, 4 gold"));
        // Empty-handed, building is refused with the bill.
        if site.near(p.x, p.y) {
            let e = w
                .apply(
                    me,
                    &Command::Build {
                        site: "grey spire".into(),
                    },
                )
                .unwrap();
            assert!(e.starts_with("x Grey Spire still needs"), "{e}");
        }
        // With the materials in hand, the work starts and the spire rises.
        {
            let p = w.player_mut(me).unwrap();
            for (r, n) in Form::Spire.cost() {
                add(&mut p.inventory, r, *n);
            }
        }
        w.apply(
            me,
            &Command::Build {
                site: "Grey Spire".into(),
            },
        )
        .unwrap();
        for _ in 0..120 {
            w.step();
            if w.place("Grey Spire").unwrap().built() {
                break;
            }
        }
        let site = w.place("Grey Spire").unwrap().clone();
        assert!(site.built(), "{site:?}");
        assert!(w.player(me).unwrap().inventory.is_empty());
        assert!(w
            .events
            .iter()
            .any(|e| e.text.starts_with("raised Grey Spire")));
        assert_eq!(w.player(me).unwrap().task, Task::Idle);
        assert!(w.player(me).unwrap().level("building") >= 1);
        assert!(w.describe(me).contains("Grey Spire (") && w.describe(me).contains(", spire)"));
        // Walking to it lands at its door, not inside it.
        w.apply(
            me,
            &Command::MoveTo {
                target: "Grey Spire".into(),
            },
        )
        .unwrap();
        for _ in 0..20 {
            w.step();
        }
        let p = w.player(me).unwrap();
        assert!(!site.covers(p.x, p.y) && site.near(p.x, p.y));
        // A banner still costs nothing and stands at once.
        let camp = Command::FoundPlace {
            name: "Camp".into(),
            description: "d".into(),
            resource: None,
            skill: None,
            form: Form::Banner,
            style: None,
        };
        assert!(w.apply(me, &camp).unwrap().contains("founds Camp"));
        assert!(w.place("Camp").unwrap().built());
        // Forms parse from what people call them.
        assert_eq!(Form::parse("wizard's tower"), Some(Form::Spire));
        assert_eq!(Form::parse("Smithy"), Some(Form::Forge));
        assert_eq!(Form::parse(""), Some(Form::Banner));
        assert_eq!(Form::parse("nonsense"), None);
    }

    #[test]
    fn a_building_never_cuts_the_map_and_its_founder_can_tear_it_down() {
        let mut w = World::new(5);
        let me = w.join("Ada");
        // Stand on the ford — the one crossing — and found a house.
        let ford = w.place("River Ford").unwrap().clone();
        {
            let p = w.player_mut(me).unwrap();
            p.x = ford.x;
            p.y = ford.y;
        }
        let house = Command::FoundPlace {
            name: "Ford House".into(),
            description: "d".into(),
            resource: None,
            skill: None,
            form: Form::House,
            style: None,
        };
        w.apply(me, &house).unwrap();
        let hp = w.place("Ford House").unwrap().clone();
        for ty in hp.y..hp.y + 2 {
            for tx in hp.x..hp.x + 2 {
                assert_ne!(w.tile(tx, ty), Tile::Road, "never on a road");
            }
        }
        // Both banks still reach Town.
        let town = w.places[0].door();
        assert!(w.path_step((2, ford.y), town).is_some());
        assert!(w.path_step((W - 2, ford.y), town).is_some());
        // Only the founder tears it down; seeded places are nobody's.
        let other = w.join("Bea");
        let tear = Command::Abandon {
            site: "ford house".into(),
        };
        assert!(w
            .apply(other, &tear)
            .unwrap()
            .starts_with("x Ford House is not"));
        assert!(w
            .apply(
                me,
                &Command::Abandon {
                    site: "Town".into()
                }
            )
            .unwrap()
            .starts_with("x Town is not"));
        assert!(w
            .apply(me, &tear)
            .unwrap()
            .contains("tears down Ford House"));
        assert!(!w.places.iter().any(|p| p.name == "Ford House"));
    }

    #[test]
    fn giving_wants_and_making_things() {
        let mut w = World::new(5);
        let me = w.join("Ada");
        w.apply(
            me,
            &Command::CreateNpc {
                name: "Old Wren".into(),
                persona: "A forager.".into(),
            },
        )
        .unwrap();
        // Wren wants fish for gold, on repeat; only her maker may say so.
        let want = Command::SetWant {
            npc: "Wren".into(),
            item: "fish".into(),
            amount: 2,
            reward: vec![("gold".into(), 1)],
            repeat: true,
            words: "a coin a pair".into(),
        };
        assert!(w
            .apply(me, &want)
            .unwrap()
            .contains("now wants 2 fish for 1 gold"));
        let other = w.join("Bea");
        assert!(w
            .apply(other, &want)
            .unwrap()
            .starts_with("x Old Wren is not"));
        // Empty-handed giving is refused; with fish, she takes them and pays.
        let give = |n: Option<u32>| Command::Give {
            item: "fish".into(),
            amount: n,
            to: "Wren".into(),
        };
        assert!(w
            .apply(me, &give(None))
            .unwrap()
            .starts_with("x Ada is not carrying"));
        add(&mut w.player_mut(me).unwrap().inventory, "fish", 3);
        let r = w.apply(me, &give(Some(2))).unwrap();
        assert!(r.contains("Old Wren gives Ada 1 gold"), "{r}");
        let p = w.player(me).unwrap();
        assert_eq!(count(&p.inventory, "fish"), 1);
        assert_eq!(count(&p.inventory, "gold"), 1);
        let wren = w.npcs[0].clone();
        assert_eq!(count(&wren.holds, "fish"), 2);
        assert_eq!(
            wren.want.as_ref().unwrap().given,
            0,
            "a standing trade resets"
        );
        assert!(w
            .speeches()
            .iter()
            .any(|s| s.text.starts_with("*hands over")));
        assert!(w
            .describe(me)
            .contains("wants 2 fish for 1 gold (0/2, standing)"));
        // Passing by with fish, she calls out — once, and not while waiting.
        w.take_speeches();
        w.step();
        assert_eq!(w.speeches().len(), 1);
        assert!(w.speeches()[0].text.starts_with("*comes near"));
        w.step();
        assert_eq!(w.speeches().len(), 1);
        // Making things takes a workshop and the materials.
        let craft = Command::Craft {
            item: "Fish Lantern".into(),
            description: "A lantern that smells of the river.".into(),
            from: vec![("fish".into(), 1)],
        };
        assert!(w
            .apply(me, &craft)
            .unwrap()
            .starts_with("x making things takes a workshop"));
        w.apply(
            me,
            &Command::FoundPlace {
                name: "Shed".into(),
                description: "d".into(),
                resource: None,
                skill: None,
                form: Form::Hut,
                style: None,
            },
        )
        .unwrap();
        let door = {
            let pl = w.places.iter_mut().find(|p| p.name == "Shed").unwrap();
            pl.needs.clear();
            pl.work = 100;
            pl.door()
        };
        {
            let p = w.player_mut(me).unwrap();
            p.x = door.0;
            p.y = door.1;
        }
        let r = w.apply(me, &craft).unwrap();
        assert!(r.contains("makes a fish lantern at Shed"), "{r}");
        let p = w.player(me).unwrap();
        assert_eq!(count(&p.inventory, "fish lantern"), 1);
        assert_eq!(count(&p.inventory, "fish"), 0);
        assert_eq!(w.items.len(), 1);
        assert!(w
            .describe(me)
            .contains("1 fish lantern (A lantern that smells"));
        // And a made thing can be handed to another player nearby.
        let (ax, ay) = w.player(me).unwrap().pos();
        {
            let b = w.player_mut(other).unwrap();
            b.x = ax + 1;
            b.y = ay;
        }
        let r = w
            .apply(
                me,
                &Command::Give {
                    item: "fish lantern".into(),
                    amount: None,
                    to: "Bea".into(),
                },
            )
            .unwrap();
        assert!(r.contains("gives 1 fish lantern to Bea"), "{r}");
        assert_eq!(
            count(&w.player(other).unwrap().inventory, "fish lantern"),
            1
        );
        assert_eq!(
            goods("2 fish and a gold coin"),
            vec![("fish".to_string(), 2), ("gold coin".to_string(), 1)]
        );
    }

    #[test]
    fn an_npc_with_a_script_walks_speaks_and_gives() {
        let mut w = World::new(5);
        let me = w.join("Ada");
        w.apply(
            me,
            &Command::CreateNpc {
                name: "Old Wren".into(),
                persona: "A forager.".into(),
            },
        )
        .unwrap();
        let id = w.npcs[0].id;
        let home = (w.npcs[0].x, w.npcs[0].y);
        // Only her maker gives her a script.
        let script = Command::SetNpcScript {
            npc: "wren".into(),
            source: "walk('3 tiles east')".into(),
        };
        let other = w.join("Bea");
        assert!(w
            .apply(other, &script)
            .unwrap()
            .starts_with("x Old Wren is not"));
        assert!(w.apply(me, &script).unwrap().contains("has a script now"));
        assert_eq!(w.npc_scripted_idle(), vec![id]);
        // A run: she sets off, and walks there over the next ticks.
        let r = w
            .npc_ran(
                id,
                vec![Command::MoveTo {
                    target: "3 tiles east".into(),
                }],
                Value::Null,
                "",
            )
            .unwrap();
        assert!(r.contains("sets off"), "{r}");
        assert!(w.npc_scripted_idle().is_empty(), "walking, and just ran");
        for _ in 0..6 {
            w.step();
        }
        let n = w.npc(id).unwrap().clone();
        assert_eq!(n.task, Task::Idle);
        assert!(n.x > home.0, "moved east: {:?} from {home:?}", (n.x, n.y));
        for _ in 0..NPC_SCRIPT_REST {
            w.step();
        }
        assert_eq!(w.npc_scripted_idle(), vec![id], "rested, she runs again");
        // Home is a place she knows; speech is a voice; giving needs someone near.
        w.npc_ran(
            id,
            vec![
                Command::Say {
                    text: "Back to the well.".into(),
                },
                Command::MoveTo {
                    target: "home".into(),
                },
            ],
            obj! {"trips" => 1},
            "log line",
        )
        .unwrap();
        assert!(w
            .events
            .iter()
            .any(|e| e.kind == "voice" && e.text.contains("Back to the well")));
        assert!(w
            .events
            .iter()
            .any(|e| e.kind == "script" && e.text == "log line"));
        assert_eq!(w.npc(id).unwrap().memory.get("trips").as_u32(), Some(1));
        for _ in 0..8 {
            w.step();
        }
        assert_eq!((w.npc(id).unwrap().x, w.npc(id).unwrap().y), home);
        add(
            &mut w.npcs.iter_mut().find(|n| n.id == id).unwrap().holds,
            "gold",
            2,
        );
        {
            let p = w.player_mut(me).unwrap();
            p.x = home.0 + 1;
            p.y = home.1;
        }
        let r = w
            .npc_ran(
                id,
                vec![Command::Give {
                    item: "gold".into(),
                    amount: Some(1),
                    to: "Ada".into(),
                }],
                Value::Null,
                "",
            )
            .unwrap();
        assert!(r.contains("gives 1 gold to Ada"), "{r}");
        assert_eq!(count(&w.player(me).unwrap().inventory, "gold"), 1);
        // A world with a walking, scripted NPC survives a save.
        let back = World::from_json(&w.to_json()).unwrap();
        assert_eq!(back.npc(id).unwrap().script, w.npc(id).unwrap().script);
        assert_eq!(back.npc(id).unwrap().home, home);
    }
}
