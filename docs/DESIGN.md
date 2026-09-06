# project cqs — design notes

*2026-09-04. The first pass, written after reading the lineage. Everything here is a
hypothesis until the code says otherwise.*

## What it is

A persistent shared world where every character is piloted by words. You type what
you want your character to do; a model turns that into one legal move; the world
applies it and keeps ticking. It is a text adventure with a display instead of a
paragraph, and with other people in it.

The display is a window, not the controller. That is the inversion that makes this
scale where a three.js MMO would not: the client renders state and sends sentences.
It never simulates, never predicts, never owns anything.

## The lineage (why this is the next step)

Each project taught one thing that this one keeps.

| project | what it proved | what it cost | what cqs keeps |
|---|---|---|---|
| **tempo-x402** | agents can write Rust, compile it to wasm, and hot-swap it into a running system; framebuffer cartridges (`x402_tick` / `x402_get_framebuffer`) | 116K LOC, a compiler in the loop, no way to reason about what a cartridge may do | hot-swappable display programs — but as data, not binaries |
| **localharness** | one crate, native + wasm32; a model-agnostic seam; sandboxed `rustlite` cartridges in a worker with a framebuffer and fractal `host::compose`; every Gemini wire gotcha (signatures, blocked frames, CRLF SSE) | the compiler is the heavy part; a browser-side Rust subset is still a compile step | the gemini wire facts (verified again live here), the "the display runs programs" idea |
| **litelite / metabolite** | purpose-sized total languages: fuel = a termination proof, a capability table = a complete effect bound; zero deps; a ≤5K-line constitution; a world that is one reproducible integer | nothing — this is the physics | the display language will be a litelite language; the world is deterministic from seed + ledger; a LOC budget is the design |
| **vanish** | the whole agent in a Web Worker, the working tree durable in OPFS, `protocol.rs` shared by both halves so a mismatch is a build error | serverless was the wrong shape for an agent loop (ARCHITECTURE.md says why) | one protocol type shared by client and server; a build that reloads itself |
| **callosa** | no application JavaScript at all; a relay that introduces peers and then leaves the data path; a 12-byte frame header with version, opcode, request id | the CPU beat the GPU at that size, and that was the honest result | zero-JS client discipline; versioned frames; request ids so late replies never resolve the wrong step |
| **hollowtide / rust-wasm-engine** | a server-authoritative 20 Hz Rust MMO loop; a zero-dep `std::net` server that syncs players | hollowtide was a v0 | the server-authoritative shape, and proof that the server can be dependency-free |
| **chat2game** | the audience *is* the population; PITCH → VOTE → PATCH → LIVE; a prefix and a quorum; "if it isn't a feature it's content"; skills as the reason to be there; profiles keyed on channel id; loopback OAuth with PKCE | the model rewrote a 24 KB HTML file each round — the artefact was code, not data | the audience loop, the ladder economy, `!mine` idling, the YouTube facts, the OAuth code |
| **Tiny Empires** (`trend`, local) | 146K LOC of pure Rust/wasm game; `core`/`client` split; `Command` as the wire and "the client is a stale mirror"; seeded deterministic sim; software framebuffer at integer scale; the meta/match/view layers; the autonomous 90-minute loop that built it | it is single-player, because a hard constraint forbade a server, and a shared world needs one place where `apply` runs in order | the `Command`/`apply` discipline, the framebuffer conventions, the dev loop, and specific systems (terrain, A*, gather/forge) lifted one at a time when needed |

Fibonacci: cqs = Tiny Empires' sim discipline + chat2game's audience loop +
litelite's display language + hollowtide's server shape + localharness's wire
knowledge. Metabolite's constitution keeps it from becoming a fourth 120K-line repo.

## The loop

```
words ──▶ pilot (model) ──▶ one Command ──▶ world.apply ──▶ world.step (1–2 Hz)
                                                   │
             display (wasm client) ◀── view ◀──────┘
```

Three decisions, each of which is the cheap one and also the right one:

**The model never touches state.** It chooses *which* legal move; the world decides
*whether* it is legal. The command set is exactly what a keyboard would drive, which
is exactly Tiny Empires' `Command` enum and `apply_command`. A model that is jailbroken
by a player's prompt can, at worst, pick a different legal move for that player's own
character. The trust boundary is an enum.

**The pilot is stateless.** There is no chat history, no compaction, no memory tool.
The world *is* the memory: every prompt is one request carrying the character's
current view (~200 tokens) plus the words, and the answer is one function call.
That is ~250 tokens on `gemini-3.5-flash-lite`, under a second, and — because the
system prompt and tool declarations never change — cacheable. Two players who say
the same thing in the same situation get the same move, which is what a game wants.

**Characters keep working between prompts.** One sentence sets a task that runs for
minutes, like RuneScape idling; the tick rate is 1–2 Hz and the model's latency
disappears behind the walk. A prompt is an *intent*, not a frame.

## Topology

**An authoritative native Rust server, and a static wasm client at cqs.gg.**

Why not the static-only shape Tiny Empires used: a shared world needs one process
where `apply` runs in order, and the model key has to live on the server (YouTube
viewers have no key; per-player budgets need one owner). Vercel Functions cannot hold
a world across calls without an external store, and that store was rejected once
already ("dirty javascript"). So:

- **Server** — one Rust binary on Railway or Fly (both already in use). Start
  dependency-free like metabolite/rust-wasm-engine: `std::net`, hand-rolled HTTP,
  **SSE out, POST in**. A 1–2 Hz text game does not need WebSocket framing; SSE is
  already decoded by this repo's `gemini::sse` and the browser handles reconnects.
  The world lives behind one mutex; the pilot runs on a thread pool so a slow model
  call never blocks a tick. Move to WebSockets only if latency says so.
- **Client** — wasm, no JS beyond the loader, a software framebuffer at integer
  scale exactly as Tiny Empires does it (`fill_rect` / `line` / `draw_string` /
  `present`). It renders views and sends sentences. The localharness/Tiny Empires
  cartridge conventions mean the renderer could later *be* a display program.
- **Persistence** — an append-only ledger of `(tick, player, Command)` plus periodic
  snapshots. `state = fold(ledger)` (metabolite, MIWE). The save format is the
  replay; crash recovery is a replay; an audit is a replay.
- **Protocol** — one `protocol.rs` shared by server and client (vanish), versioned
  frames with request ids (callosa).

## The display runs programs

This is the part Kyle called the hard part, and the part that makes the game able to
build itself. Three generations:

1. **tempo-x402**: wasm cartridges compiled by a toolchain, hot-swapped at runtime.
   Works; needs rustc in the loop; a cartridge is opaque.
2. **localharness `rustlite`**: a Rust-subset compiler to wasm running in the browser.
   Works; the compiler is most of the weight; still a compile step.
3. **cqs**: a purpose-sized *display language* interpreted directly. No compiler.
   Fuel-bounded per frame (a frame provably ends). A capability table that is
   exactly the draw, input, and view calls (a program provably touches nothing else).
   A byte budget. A program is a few hundred bytes of text that travels in the same
   JSON as everything else, is verified by grammar + capabilities on arrival, and
   cannot hang a tab or escape it. litelite's `fuellite`, `caplite`, `parselite`,
   `lexlite` and `diaglite` are the kernel; `applite` is the closest sibling.

What runs there: the world view itself, panels, minimaps, a place's minigame (the
fishing minigame at River Ford, written by the model when River Ford was created),
each player's own "screen". The server ships programs to clients as data. The
model writes the display program for a new place at the moment the place is made.
The client's rendering is data-driven, so the game grows screens without a deploy.

"Beyond hot-swappable" is: there is nothing to swap, because programs are values.
"Beyond browser compiling" is: there is nothing to compile, because the language
is the size of what needs saying.

Concretely: `crates/display` — a small stack VM (or expression language) over a
framebuffer surface with a fuel budget and a fixed capability table. The wasm
client hosts it over a real framebuffer; the CLI hosts the same VM over an ASCII
surface, which is how it gets tested.

## The audience

From chat2game, verbatim where possible:

- Read YouTube live chat over InnerTube (no key, no quota). A viewer plays by typing
  `!` and words; the server pilots their character. Profiles are keyed on the channel
  id, never the display name.
- Claiming a character on the site: a one-time code shown in chat, or Google OAuth
  (chat2game's loopback PKCE flow exists and is Rust).
- Rate limits per channel id. A pilot call is on the order of $0.0001, so a chat of
  hundreds is affordable; a chat of thousands needs a queue and a coarser tick.
- PITCH → VOTE → PATCH becomes: world edits (found a place, name a thing, build) are
  `Command`s too — proposed by players, quorum-gated, applied by the same `apply`.
  "If it isn't a feature, it's content." Nothing decorative; everything is in the world.

## Self-building, safely

- **Content** is structured output (`responseSchema`) into the same types the sim
  uses — a `Place`, an NPC, a quest — validated before insertion. Free text never
  enters state.
- **Display programs** for new content are generated, then verified (grammar, fuel,
  capabilities) before they ship. A program that fails verification is a diagnostic,
  not a bug.
- **Two loops, two artefacts**: the autonomous dev loop from Tiny Empires
  (`LOOP.md`, the tasks ledger) keeps building the *code*; the in-game loop builds
  the *content*. They must not be confused: the game does not edit its own Rust.

## Constitution

Borrowed from metabolite, because the parents died at 120K lines.

1. Zero dependencies in the core crates (`gemini` core, `world`, `display`). Deps only
   in transports and, if unavoidable, the server. Today the whole workspace has one:
   `ureq`, for TLS.
2. One language. No JavaScript beyond the wasm loader. No Python.
3. Every state change is a `Command`. The model never holds the pen.
4. The world is deterministic from `(seed, ledger)`.
5. A line budget, enforced by a script: sim + display language + server ≤ 15K lines.
   When it is hit, something is cut.

## What exists (2026-09-04)

- `crates/gemini` — the API from first principles. `json.rs` (a `Value`, parser,
  writer), `sse.rs` (an incremental decoder), `lib.rs` (Content/Part with signatures
  preserved, Request → body/HTTP, Response, a stream folder, errors). Zero deps.
  `native` feature: `ureq` client with `generate`, `stream`, `models`, `.env` loading.
  `web` feature: `fetch` client for wasm32. Every wire fact was probed live.
- `crates/world` — a 32×18 deterministic map with six seeded places, characters with
  tasks, inventories, banks and free-form skills; `Command` (move, gather, bank, say,
  look, stop, save/run recipe, found place, create NPC) + `plan`/`apply` + `step`;
  plans queue and loop; `describe` (the text view) and `ascii` (the terminal
  display); `pilot` (system prompt, ten tool declarations, calls → `Command`s, the
  NPC voice request, and a keyword fallback for offline play).
- `crates/cqs` — the terminal game with a real-time clock on its own thread.
  `cqs`, `cqs --offline`, `cqs --script "..." "..."`, `--tps`.

## Milestones

- **M1** *(done)* — the pilot loop at the terminal, against the live API.
- **M1.5** *(done)* — real time; prompts as chained plans; recipes that loop;
  founding places with invented resources; NPCs with voices.
- **M2** — the server: one binary, ledger + snapshots, SSE view stream, POST prompt,
  Google OAuth (`sub` → character); the CLI becomes a client; deploy to Railway.
- **M3** — the wasm client at cqs.gg: framebuffer display, a prompt box, no JS.
- **M4** — `crates/display`: the fuel-bounded display language; the first
  model-written place program, verified before it ships.
- **M5** — YouTube chat players (channel id → character) and the claim flow.
- **M6** — items, quests, regions as schema-checked content; proposals with quorum.
- **Later** — on-chain identity and currency, NFT items, tokenized settlement,
  Stripe credits.

## Decisions (Kyle, 2026-09-04)

- **Model:** `gemini-3.8-flash` with `thinkingLevel: low`. Probed: default thinking
  costs ~90 thought tokens and doubles latency for a pilot call; `low` gives zero
  thought tokens at ~650 ms; `minimal` is rejected by 3.8. The model returns ordered
  multi-step function calls natively, so a chained prompt needs no wrapper.
- **Identity:** Google OAuth (the server-side web flow; the `sub` claim is the stable
  id, exactly as chat2game keys profiles on the channel id). On-chain identity, an
  in-game currency, NFT items and a tokenized settlement layer come later; so does
  Stripe onboarding for credits — localharness already has working card payments to
  lift. None of that is in the way of the game being fun.
- **Tick:** real time, 1/s, the whole world, whether or not anyone is typing.
- **Scope:** Tiny Empires stays its own thing. cqs grows from a blank slate and takes
  ideas, not code. The point is *user-generated AI content*: players develop the game
  and play it with the same words — designing places, items, quests, regions, NPCs —
  and prompts double as scripts, chains of steps ("walk to the forest, chop trees,
  bank") that can be named and re-run like recipes.
- **What matters most:** fun, novel, hyper-creative, emergent, infinite potential.
  Every design choice below is measured against that.

## Prompts as scripts

A prompt is a *plan*: the model answers with one function call per step, in order.
The world queues them and runs each as the previous finishes — `gather` with an
amount completes, `bank` walks to town and deposits, `say` fires on the way. The
last plan a player ran can be named (`save this as woodrun`) and run again, once or
forever (`keep doing my woodrun`). A recipe is just a `Vec<Command>` on the player,
so it is data, replayable, and — later — tradeable: a recipe is a *receipt* for a
routine, which is where the on-chain idea reconnects.

## Players build the world

Two tools today, both validated by the world and never invented by it:

- `found_place(name, description, resource?, skill?)` — a named place exactly where
  the player stands, with any resource word they can imagine ("mushrooms",
  "foraging"). It is visible to everyone, gatherable by everyone, and credited to
  its founder in the view.
- `create_npc(name, persona)` — a person or creature with a persona. When a player
  speaks near one, a second model call (the *voice*, prose only, nothing changes)
  answers in character. The NPC is a little program written in English.

Next in the same vein: items (crafted from what places yield), quests (an NPC's ask
with a checkable objective and a reward), regions (a founded place that opens a new
map), and in-place minigames (the display language, M4).

## Deployment (2026-09-04): Vercel, without a server

Kyle asked for Vercel first, to test before any auth. That changes the topology
from "an authoritative process" to "a function and a ledger", and it turns out
the ledger design carries it:

- **`api/world.rs`** — one Rust function on Vercel's official Rust runtime
  (`vercel_runtime` 2.x, zero-config from `api/*.rs`). `GET ?token=` is a view,
  `POST {token, name?, words?}` joins and speaks. It is a thin adapter over
  `crates/host`.
- **`crates/host`** — loads the latest snapshot and the entries after it, folds
  them, advances the world to *now*, does the request's work, appends what
  happened, and stores a new snapshot if no one else wrote in between. Nothing
  lives in memory between requests, so any number of instances agree.
- **`crates/world/ledger.rs`** — `Realm = fold(entries)`. Time is the entries'
  timestamps: one tick per second between events, capped at 600 per gap, so an
  empty world sleeps instead of replaying a week. Deterministic, so a snapshot
  is only ever an optimisation and any prefix is a valid one.
- **Neon Postgres** (free plan, provisioned through the Vercel Marketplace) is
  the store — chosen because its SQL-over-HTTPS endpoint needs no driver: the
  host speaks it with the same `ureq` it already has. Two tables: `ledger`
  (append-only, serial id = order) and `snapshot` (one row, moves forward only).
  Upstash Redis was the first candidate; the marketplace offers it only on paid
  plans, so it was not provisioned without asking.
- **`crates/web`** — the browser client, Rust through `web-sys`; the page's only
  JavaScript is the wasm loader. Identity is a random token in `localStorage`
  plus a chosen name — the temporary persistence until Google OAuth lands. The
  server only ever sees a token, so the login swap is a client-side change.
- **NPC voices** are ledger entries too (`NpcSays`), keyed to the tick of the
  speech they answer, so a replay does not re-ask the model and does not answer
  twice.

What this costs: a request folds the tail of the ledger (microseconds) and makes
two or three HTTPS calls to Neon (tens of milliseconds each), plus the model.
What it buys: no process to keep alive, no server to pay for, and a world whose
whole history is a table anyone can replay.

## Scripts (2026-09-05): Lua on piccolo

Kyle asked "what about Lua?" and the answer was yes, for one decisive reason: the
model already writes Lua fluently and players know it, so no invented language
would be written as well. The VM is **piccolo**, a pure-Rust, sandboxed,
fuel-metered Lua; it is the first dependency the game logic has taken, and it
lives in its own crate (`crates/script`) so `world` stays zero-dep.

How it fits the ledger:

- A script is a `Command` (`SetScript`) and lives on the player with a `memory`
  table. The pilot has a `script` tool; agents can `POST {script: "..."}`.
- The **host** runs a script only when its character is idle (nothing queued,
  no recipe on repeat, not already run this tick), with 200k instructions of
  fuel and at most eight steps per run. It sees `me`, `places`, `people`,
  `tick`, `memory`, and calls `walk gather bank say found npc near dist log`.
- What it decides is recorded as a `Ran` entry (steps, new memory, log or
  error). Replay never runs Lua; a script's effects are data like everything
  else. The world remains the only authority on what is legal.

Also in this round: direct `cmds` in the API (no model needed), `GET ?doc`
(the API describes itself for agents arriving cold), NPC voices answer the
character actually addressed within two tiles and never twice, and the pilot
no longer narrates what the game cannot do — a wish for a shop becomes a place
and a person, not an apology.

Not GitHub: persistence is the Neon ledger, and in-game programs (scripts, later
NPC behaviour and display programs) are ledger data, not repo files. Writing
the repo from inside the game would only matter for a vanish-style loop that
rewrites the Rust, and Vercel's git deploy already turns a commit into a build.

## Buildings and a camera (2026-09-05): taking more from Tiny Empires

Kyle said "make a wizard's tower" and got a flag with the words on it, on a
map that showed the whole world at once. Both were the same mistake: the
world had places but no things, and the display had a map but no window.

**Forms.** A place now has a form. `banner` is the old spot — free, instant,
a name on the ground. The rest are buildings: hut, house, hall, tower, spire
(the wizard's tower), forge, mill, shrine, well. A building has a footprint
nobody walks through (paths go around it, the founder steps off it, its door
is the tile below its front), a bill of materials in resources the seeded
world yields, and ticks of work. Founding marks the site out and says what it
needs; `build` walks there, hands over what is carried, and works until it
stands. Materials must be *carried*, not banked, so a tower is a trip: gather
40 stone, carry it to the site, build. The pilot's system prompt carries the
cost table so one sentence can become found → gather → build.

**The window.** The client is a sixteen-tile square at 48 px a tile that
follows your character (a spectator watches Town), with a minimap in the
corner. This is Tiny Empires' field, not its overworld: you see the place
you are in, and the map is something you consult.

**The art.** `crates/web/src/arch.rs` draws buildings the way Tiny Empires'
`architecture.rs` does — walls with courses on a plinth, a shingled roof with
a sunlit pitch and a shaded one, a lit doorway — and gives each form a mark
outside the shell (a shaft with an orb, a stack, sails, a yard), because two
buildings that share a silhouette are one building at reading distance. A
style word chooses the materials: stone, dark (iron teeth on the ridge, like
the Horde's keeps), white, red, blue, gold, mossy, purple, timber, or any
other word for a roof colour of its own. A site waiting for materials is
stakes and rope with a signboard; a site being built rises out of the ground
behind a scaffold, clipped at how much of it stands.

Not lifted, on purpose: Tiny Empires' factions, combat, and training. cqs
has one civilisation and the difference between buildings is who founded
them and what they said.

## Things change hands (2026-09-05): give, wants, made things

The Nettle episode: the voice invented a quest ("bring me a fresh catch")
and the world had no way to let it happen. Now it does, in three verbs.

- **give** hands something carried to a person within two tiles, NPC or
  player. NPCs keep what they are given (`holds`), and the voice knows it.
- **wants** are a quest in one line, set by whoever made the NPC: what it
  wants, how many, what it hands back, and whether the trade stands or ends.
  Giving the wanted thing counts toward it; meeting it pays the reward into
  the giver's pack and cues the voice to react. An NPC with a want *hails*
  anyone who walks by carrying it (a cue speech, at most once in forty
  ticks), so a quest finds you.
- **craft** makes a named thing from carried materials at any built
  building. The model names it, describes it, and says what it is made of;
  the world checks the materials are in hand and the workshop exists. Made
  things are words in packs like any resource — carried, banked, given,
  wanted — and the world keeps a catalogue of what they are.

Cues: a speech whose text starts with `*` is something done, not said
("*hands over 2 fish"); the voice prompt renders it as an action line. No
new ledger kind was needed — a cue is a `Speech` and a reaction is an
`NpcSays`, as before.

## A bigger world (2026-09-05)

With a window instead of a map, the world can be larger than the screen.
It is 48×48 now: the same river, ford, hills, quarry and forest, plus a
lake, a wood across the river with Gold Creek in it, a ridge in the east
and a reed marsh in the south — one more seeded place, `reeds/foraging`,
because a marsh is the kind of place a player wants to found something
at. `cargo run -p cqs --example map -- <seed>` prints a layout. NPCs with
a want carry a quest marker in the display, so a trade finds you.

## NPCs with lives (2026-09-05)

A character someone made can be given a standing Lua script by its maker,
with the same API a player's script has: `me`, `people`, `places`, `tick`,
`memory`; `walk`, `say`, `give`, `log`, `near`, `dist`; and `walk("home")`
to go back to where they were made. NPCs walk the same paths players do
(around buildings, not through them), speak as voices, and hand over what
they hold. The host runs an idle NPC's script every ten ticks and records
what it decided as an `NpcRan` entry, so replay never runs Lua. "Nettle
wanders the bank and hails anyone carrying fish" is now one sentence.

## Offers (2026-09-05): players as shops

The same `Want` an NPC carries can sit on a player: what their character
buys from other players and what it pays out of its own pack. Giving that
thing to them pays automatically; if they cannot pay, the goods come back
and the gift is refused, so nobody is robbed by an empty shop. With NPC
wants, this is the whole economy so far: bounties, trades, shops, all
authored in a sentence, all settled by the world. The view also carries
the time of day now (a day is 1200 ticks, and the display darkens through
the second half of it), so voices and pilots know whether it is dusk.
