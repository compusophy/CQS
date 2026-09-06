//! The display: a software framebuffer the scene is rasterized into, and the
//! rasterizers that do it. No canvas drawing API, no fonts from the browser —
//! rects, discs, lines, dither and a bitmap face, exactly as Tiny Empires
//! draws. The browser only ever receives finished pixels.
//!
//! The frame is a WINDOW onto the world, not the world: sixteen tiles square,
//! following your character, the way Tiny Empires' field follows its camera.
//! Buildings are drawn by `arch`; a minimap in the corner keeps the whole map
//! in view.

use gemini::Value;

use crate::arch;

/// One map tile is this many pixels.
pub const TILE: i32 = 48;
/// The window onto the world: sixteen tiles square.
pub const VIEW: i32 = 16 * TILE;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Grass,
    Water,
    Forest,
    Hill,
    Road,
    Town,
}

impl Tile {
    fn from_glyph(c: char) -> Tile {
        match c {
            '~' => Tile::Water,
            'T' => Tile::Forest,
            '^' => Tile::Hill,
            '=' => Tile::Road,
            '#' => Tile::Town,
            _ => Tile::Grass,
        }
    }
    fn base(self) -> u32 {
        match self {
            Tile::Grass => 0x4f8a3f,
            Tile::Water => 0x2c68a3,
            Tile::Forest => 0x3e7534,
            Tile::Hill => 0x8c8069,
            Tile::Road => 0xb8a27b,
            Tile::Town => 0x9a9791,
        }
    }
}

/// A place: a banner on a spot, or a building with a footprint.
#[derive(Clone, Debug, Default)]
pub struct Mark {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub resource: Option<String>,
    /// "banner", "hut", "house", "hall", "tower", "spire", "forge", "mill", "shrine", "well".
    pub form: String,
    pub w: i32,
    pub h: i32,
    pub built: bool,
    /// 0..1 of the work done, once the materials are on site.
    pub progress: f32,
    pub style: Option<String>,
}

impl Mark {
    pub fn built(&self) -> bool {
        self.built
    }
}

#[derive(Clone, Debug, Default)]
pub struct Figure {
    pub name: String,
    pub x: i32,
    pub y: i32,
    /// "idle", "walk", "gather", "build"
    pub doing: String,
    pub resource: Option<String>,
    pub me: bool,
    pub npc: bool,
    /// An NPC with a want: a quest marker over their head.
    pub wants: bool,
}

/// Something said lately, to be drawn over whoever said it.
#[derive(Clone, Debug, Default)]
pub struct Bubble {
    pub name: String,
    pub text: String,
    #[allow(dead_code)]
    pub tick: u64,
}

/// Everything the renderer knows about one moment of the world.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub w: i32,
    pub h: i32,
    #[allow(dead_code)]
    pub tick: u64,
    pub tiles: Vec<Tile>,
    pub places: Vec<Mark>,
    pub figures: Vec<Figure>,
    pub speech: Vec<Bubble>,
}

impl Scene {
    pub fn from_json(v: &Value) -> Option<Scene> {
        let w = v.get("w").as_i64()? as i32;
        let h = v.get("h").as_i64()? as i32;
        let mut tiles = Vec::with_capacity((w * h) as usize);
        for row in v.get("tiles").as_arr() {
            let row = row.as_str()?;
            tiles.extend(row.chars().map(Tile::from_glyph));
        }
        if tiles.len() != (w * h) as usize {
            return None;
        }
        let places = v
            .get("places")
            .as_arr()
            .iter()
            .map(|p| Mark {
                name: p.get("name").to_text(),
                x: p.get("x").as_i64().unwrap_or(0) as i32,
                y: p.get("y").as_i64().unwrap_or(0) as i32,
                resource: p.get("resource").as_str().map(str::to_string),
                form: p
                    .get("form")
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("banner")
                    .to_string(),
                w: p.get("w").as_i64().unwrap_or(1).max(1) as i32,
                h: p.get("h").as_i64().unwrap_or(1).max(1) as i32,
                built: p.get("built").as_bool().unwrap_or(true),
                progress: p.get("progress").as_f64().unwrap_or(1.0) as f32,
                style: p.get("style").as_str().map(str::to_string),
            })
            .collect();
        let mut figures: Vec<Figure> = v
            .get("players")
            .as_arr()
            .iter()
            .map(|p| Figure {
                name: p.get("name").to_text(),
                x: p.get("x").as_i64().unwrap_or(0) as i32,
                y: p.get("y").as_i64().unwrap_or(0) as i32,
                doing: p.get("doing").to_text(),
                resource: p.get("resource").as_str().map(str::to_string),
                me: p.get("me").as_bool().unwrap_or(false),
                npc: false,
                wants: false,
            })
            .collect();
        figures.extend(v.get("npcs").as_arr().iter().map(|n| Figure {
            name: n.get("name").to_text(),
            x: n.get("x").as_i64().unwrap_or(0) as i32,
            y: n.get("y").as_i64().unwrap_or(0) as i32,
            doing: n.get("doing").as_str().unwrap_or("idle").to_string(),
            resource: None,
            me: false,
            npc: true,
            wants: n.get("wants").as_str().is_some(),
        }));
        let speech = v
            .get("speech")
            .as_arr()
            .iter()
            .map(|s| Bubble {
                name: s.get("name").to_text(),
                text: s.get("text").to_text(),
                tick: s.get("tick").as_f64().unwrap_or(0.0) as u64,
            })
            .collect();
        Some(Scene {
            w,
            h,
            tick: v.get("tick").as_f64().unwrap_or(0.0) as u64,
            tiles,
            places,
            figures,
            speech,
        })
    }

    fn tile(&self, x: i32, y: i32) -> Tile {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            Tile::Water
        } else {
            self.tiles[(y * self.w + x) as usize]
        }
    }
}

// ---------------------------------------------------------------------------
// The framebuffer
// ---------------------------------------------------------------------------

pub struct Frame {
    pub w: i32,
    pub h: i32,
    /// RGBA, row-major, exactly what `putImageData` wants.
    pub px: Vec<u8>,
    /// Rows above this are not painted: how a building rises from the ground.
    pub clip_top: i32,
}

impl Frame {
    pub fn new(w: i32, h: i32) -> Frame {
        Frame {
            w,
            h,
            px: vec![0; (w * h * 4) as usize],
            clip_top: 0,
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < self.clip_top || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        self.px[i] = (rgb >> 16) as u8;
        self.px[i + 1] = (rgb >> 8) as u8;
        self.px[i + 2] = rgb as u8;
        self.px[i + 3] = 255;
    }

    /// Blend `rgb` over the pixel with `alpha` in 0..=255.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, rgb: u32, alpha: u32) {
        if x < 0 || y < self.clip_top || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let mix = |old: u8, new: u32| ((old as u32 * (255 - alpha) + new * alpha) / 255) as u8;
        self.px[i] = mix(self.px[i], rgb >> 16 & 255);
        self.px[i + 1] = mix(self.px[i + 1], rgb >> 8 & 255);
        self.px[i + 2] = mix(self.px[i + 2], rgb & 255);
        self.px[i + 3] = 255;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        for yy in y.max(self.clip_top)..(y + h).min(self.h) {
            for xx in x.max(0)..(x + w).min(self.w) {
                self.put(xx, yy, rgb);
            }
        }
    }

    pub fn shade_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32, alpha: u32) {
        for yy in y.max(self.clip_top)..(y + h).min(self.h) {
            for xx in x.max(0)..(x + w).min(self.w) {
                self.blend(xx, yy, rgb, alpha);
            }
        }
    }

    pub fn disc(&mut self, cx: i32, cy: i32, r: i32, rgb: u32) {
        for yy in -r..=r {
            for xx in -r..=r {
                if xx * xx + yy * yy <= r * r {
                    self.put(cx + xx, cy + yy, rgb);
                }
            }
        }
    }

    pub fn shade_disc(&mut self, cx: i32, cy: i32, r: i32, rgb: u32, alpha: u32) {
        for yy in -r..=r {
            for xx in -r..=r {
                if xx * xx + yy * yy <= r * r {
                    self.blend(cx + xx, cy + yy, rgb, alpha);
                }
            }
        }
    }

    /// A ring one pixel wide.
    pub fn ring(&mut self, cx: i32, cy: i32, r: i32, rgb: u32) {
        for yy in -r..=r {
            for xx in -r..=r {
                let d = xx * xx + yy * yy;
                if d <= r * r && d > (r - 1) * (r - 1) {
                    self.put(cx + xx, cy + yy, rgb);
                }
            }
        }
    }

    /// Bresenham.
    pub fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, rgb: u32) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put(x0, y0, rgb);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Text in the bitmap face, `scale` pixels per dot. Returns the width drawn.
    pub fn text(&mut self, x: i32, y: i32, s: &str, rgb: u32, scale: i32) -> i32 {
        let mut cx = x;
        for c in s.chars() {
            if let Some(rows) = glyph(c) {
                for (ry, row) in rows.iter().enumerate() {
                    for (rx, b) in row.bytes().enumerate() {
                        if b == b'#' {
                            self.fill_rect(
                                cx + rx as i32 * scale,
                                y + ry as i32 * scale,
                                scale,
                                scale,
                                rgb,
                            );
                        }
                    }
                }
            }
            cx += 6 * scale;
        }
        cx - x
    }

    /// Text with a dark shadow, so it reads on any ground.
    pub fn label(&mut self, x: i32, y: i32, s: &str, rgb: u32, scale: i32) {
        self.text(x + scale, y + scale, s, 0x101418, scale);
        self.text(x, y, s, rgb, scale);
    }
}

pub fn text_width(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * 6 * scale
}

/// Break text into lines of at most `cols` glyphs, on spaces where possible.
pub fn wrap(s: &str, cols: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let word: String = word.chars().take(cols).collect();
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > cols {
            lines.push(std::mem::take(&mut cur));
            if lines.len() == max_lines {
                break;
            }
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() && lines.len() < max_lines {
        lines.push(cur);
    } else if !cur.is_empty() {
        // Ran out of lines with words left: mark the cut.
        if let Some(last) = lines.last_mut() {
            if last.chars().count() > cols - 1 {
                *last = last.chars().take(cols - 1).collect();
            }
            last.push('…');
        }
    }
    lines
}

/// A small, fast, deterministic hash: the same speckle every frame.
pub(crate) fn hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

pub(crate) fn name_hash(s: &str) -> u32 {
    s.bytes().fold(0x811C_9DC5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

/// A saturated outfit colour from a name.
fn outfit(name: &str) -> u32 {
    const PALETTE: [u32; 10] = [
        0xd9534f, 0x3b7dd8, 0x2fa36b, 0xd98f2b, 0x8e5bd6, 0x20a5a5, 0xc94f9a, 0x6f8f2a, 0xb5651d,
        0x4d6fb8,
    ];
    PALETTE[(name_hash(name) % PALETTE.len() as u32) as usize]
}

fn resource_color(r: &str) -> u32 {
    match r {
        "wood" => 0x9a6a3a,
        "iron" => 0x8a8fa0,
        "stone" => 0xb0b0b0,
        "fish" => 0x5aa7e0,
        "gold" => 0xf2c94c,
        other => {
            let h = name_hash(other);
            0x60_60_60 | ((h & 0x7f) << 16) | ((h >> 8 & 0x7f) << 8) | (h >> 16 & 0x7f)
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing the scene
// ---------------------------------------------------------------------------

/// Where a figure stands, in world pixels, interpolated between two scenes.
fn lerp_pos(prev: Option<&Figure>, cur: &Figure, t: f32) -> (i32, i32) {
    let (tx, ty) = (cur.x as f32 * TILE as f32, cur.y as f32 * TILE as f32);
    let (fx, fy) = match prev {
        Some(p) if (p.x - cur.x).abs() <= 6 && (p.y - cur.y).abs() <= 6 => {
            (p.x as f32 * TILE as f32, p.y as f32 * TILE as f32)
        }
        _ => (tx, ty),
    };
    ((fx + (tx - fx) * t) as i32, (fy + (ty - fy) * t) as i32)
}

fn earlier<'a>(prev: Option<&'a Scene>, fig: &Figure) -> Option<&'a Figure> {
    prev.and_then(|s| {
        s.figures
            .iter()
            .find(|p| p.name == fig.name && p.npc == fig.npc)
    })
}

/// The camera: on your character, else on Town, clamped to the map — and
/// a map smaller than the window sits in the middle of it.
fn camera(f: &Frame, prev: Option<&Scene>, cur: &Scene, t: f32) -> (i32, i32) {
    let (cx, cy) = if let Some(fig) = cur.figures.iter().find(|g| g.me) {
        let (px, py) = lerp_pos(earlier(prev, fig), fig, t);
        (px + TILE / 2, py + TILE / 2)
    } else if let Some(m) = cur.places.iter().find(|m| m.name == "Town") {
        (m.x * TILE + TILE / 2, m.y * TILE + TILE / 2)
    } else {
        (cur.w * TILE / 2, cur.h * TILE / 2)
    };
    let clamp = |c: i32, span: i32, view: i32| {
        if span <= view {
            (span - view) / 2
        } else {
            (c - view / 2).clamp(0, span - view)
        }
    };
    (clamp(cx, cur.w * TILE, f.w), clamp(cy, cur.h * TILE, f.h))
}

enum Thing {
    Mark(usize),
    Fig(usize),
}

/// Render one frame. `t` is how far (0..1) we are between `prev` and `cur`;
/// `ms` is a clock for animation.
pub fn draw(f: &mut Frame, prev: Option<&Scene>, cur: &Scene, t: f32, ms: f64) {
    let phase = (ms / 1000.0) as f32;
    let (cam_x, cam_y) = camera(f, prev, cur, t);

    // Ground: only the tiles the window can see, plus a rim for overhangs.
    let (tx0, tx1) = (
        cam_x.div_euclid(TILE) - 1,
        (cam_x + f.w).div_euclid(TILE) + 1,
    );
    let (ty0, ty1) = (
        cam_y.div_euclid(TILE) - 1,
        (cam_y + f.h).div_euclid(TILE) + 1,
    );
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            draw_tile(f, cur, tx, ty, tx * TILE - cam_x, ty * TILE - cam_y, ms);
        }
    }

    // Everything standing on the ground, back to front by where its feet are.
    let mut things: Vec<(i32, Thing)> = Vec::new();
    let mut at: Vec<(i32, i32)> = Vec::with_capacity(cur.figures.len());
    for (i, fig) in cur.figures.iter().enumerate() {
        let (px, py) = lerp_pos(earlier(prev, fig), fig, t);
        at.push((px - cam_x, py - cam_y));
        things.push((py - cam_y + TILE, Thing::Fig(i)));
    }
    for (i, m) in cur.places.iter().enumerate() {
        things.push(((m.y + m.h) * TILE - cam_y, Thing::Mark(i)));
    }
    things.sort_by_key(|(foot, thing)| (*foot, matches!(thing, Thing::Fig(_)) as i32));

    let mut labels: Vec<(i32, i32, String, u32)> = Vec::new();
    let mut heads: Vec<(usize, i32, i32)> = Vec::new();
    for (_, thing) in things {
        match thing {
            Thing::Mark(i) => {
                let m = &cur.places[i];
                let (x, y) = (m.x * TILE - cam_x, m.y * TILE - cam_y);
                let (w, h) = (m.w * TILE, m.h * TILE);
                if x + w < -2 * TILE || x > f.w + 2 * TILE || y + h < -3 * TILE || y > f.h + TILE {
                    continue;
                }
                if m.form == "banner" {
                    if m.name == "Town" {
                        arch::draw_structure(
                            f,
                            (x - TILE, y - TILE / 2, 3 * TILE, TILE + TILE / 2),
                            "hall",
                            None,
                            true,
                            1.0,
                            ms,
                        );
                    }
                    let color = m
                        .resource
                        .as_deref()
                        .map(resource_color)
                        .unwrap_or(0xd94a4a);
                    draw_banner(f, x, y, color);
                    labels.push((x + TILE / 2, y - 18, m.name.to_uppercase(), 0xf3f0e6));
                } else {
                    let top = arch::draw_structure(
                        f,
                        (x, y, w, h),
                        &m.form,
                        m.style.as_deref(),
                        m.built,
                        m.progress,
                        ms,
                    );
                    let (text, ink) = if m.built {
                        (m.name.to_uppercase(), 0xf3f0e6)
                    } else {
                        (format!("{} (SITE)", m.name.to_uppercase()), 0xe0b46c)
                    };
                    labels.push((x + w / 2, top - 18, text, ink));
                }
            }
            Thing::Fig(i) => {
                let fig = &cur.figures[i];
                let (px, py) = at[i];
                if px < -TILE || px > f.w + TILE || py < -2 * TILE || py > f.h + TILE {
                    continue;
                }
                let moving = earlier(prev, fig)
                    .map(|b| (b.x, b.y) != (fig.x, fig.y))
                    .unwrap_or(false)
                    && fig.doing == "walk";
                let top = draw_figure(f, fig, px, py, moving, phase);
                if fig.wants {
                    // The quest marker: a bobbing "!" beside the head.
                    let bob = ((phase * 3.0).sin() * 3.0) as i32;
                    let (mx, my) = (px + TILE / 2 + 16, top - 4 + bob);
                    f.shade_disc(mx + 1, my + 1, 8, 0x000000, 90);
                    f.disc(mx, my, 8, 0x2a2416);
                    f.disc(mx, my, 7, 0xffd23a);
                    f.fill_rect(mx - 1, my - 5, 3, 6, 0x2a2416);
                    f.fill_rect(mx - 1, my + 2, 3, 2, 0x2a2416);
                }
                heads.push((i, px + TILE / 2, top));
                let ink = if fig.me {
                    0xfff2a8
                } else if fig.npc {
                    0xd9ccff
                } else {
                    0xf3f0e6
                };
                labels.push((px + TILE / 2, top - 18, fig.name.to_uppercase(), ink));
            }
        }
    }
    // Night: a day is 1200 ticks; from dusk the world blues and dims, and
    // every character carries a little light of their own.
    let day = (cur.tick % 1200) as f32 / 1200.0;
    let dark = ((day - 0.5).abs() * 2.0 - 0.55).max(0.0) / 0.45;
    if dark > 0.0 {
        let alpha = (dark * 120.0) as u32;
        let (mw, mh) = (f.w, f.h);
        f.shade_rect(0, 0, mw, mh, 0x0b1a3a, alpha);
        for (i, _) in cur.figures.iter().enumerate() {
            let (px, py) = at[i];
            f.shade_disc(
                px + TILE / 2,
                py + TILE / 2,
                TILE,
                0xffd28a,
                (dark * 40.0) as u32,
            );
            f.shade_disc(
                px + TILE / 2,
                py + TILE / 2,
                TILE / 2,
                0xffd28a,
                (dark * 40.0) as u32,
            );
        }
    }
    // Names over everything, so a wall never hides who is behind it — and
    // never on top of each other: a name that would land on another is
    // nudged up until it has a row of its own.
    labels.sort_by_key(|(_, y, _, _)| *y);
    let mut placed: Vec<(i32, i32, i32, i32)> = Vec::new();
    for (cx, y, text, ink) in &labels {
        let w = text_width(text, 2) + 6;
        let (x0, mut y0) = (cx - w / 2, *y);
        let overlaps = |y0: i32, placed: &[(i32, i32, i32, i32)]| {
            placed.iter().any(|&(px, py, pw, ph)| {
                x0 < px + pw && x0 + w > px && y0 < py + ph && y0 + 16 > py
            })
        };
        let mut tries = 0;
        while overlaps(y0, &placed) && tries < 12 {
            y0 -= 17;
            tries += 1;
        }
        placed.push((x0, y0, w, 16));
        f.label(x0 + 3, y0, text, *ink, 2);
    }
    // Speech, over whoever said it; the newest for each speaker wins.
    let mut spoken: Vec<&str> = Vec::new();
    for b in &cur.speech {
        if spoken.contains(&b.name.as_str()) {
            continue;
        }
        if let Some((_, hx, hy)) = heads
            .iter()
            .find(|(i, _, _)| cur.figures[*i].name == b.name)
        {
            draw_bubble(f, *hx, *hy - 22, &b.text);
            spoken.push(&b.name);
        }
    }
    draw_minimap(f, cur, cam_x, cam_y, &at);
}

/// The whole map in the corner, with the window drawn on it.
fn draw_minimap(f: &mut Frame, s: &Scene, cam_x: i32, cam_y: i32, at: &[(i32, i32)]) {
    let k = 2;
    let (mw, mh) = (s.w * k, s.h * k);
    let (x0, y0) = (f.w - mw - 10, f.h - mh - 10);
    f.shade_rect(x0 - 3, y0 - 3, mw + 6, mh + 6, 0x000000, 150);
    f.fill_rect(x0 - 1, y0 - 1, mw + 2, mh + 2, 0x1b1f26);
    for ty in 0..s.h {
        for tx in 0..s.w {
            f.fill_rect(
                x0 + tx * k,
                y0 + ty * k,
                k,
                k,
                arch::shade(s.tile(tx, ty).base(), 0.8),
            );
        }
    }
    for m in &s.places {
        let c = if m.form == "banner" {
            0xf3f0e6
        } else if m.built {
            0xffd27a
        } else {
            0xe0b46c
        };
        f.fill_rect(
            x0 + m.x * k,
            y0 + m.y * k,
            (m.w * k).max(2),
            (m.h * k).max(2),
            c,
        );
    }
    for (i, fig) in s.figures.iter().enumerate() {
        let (px, py) = at[i];
        let (tx, ty) = ((px + cam_x) / TILE, (py + cam_y) / TILE);
        let c = if fig.me {
            0xfff2a8
        } else if fig.npc {
            0xb9a7d8
        } else {
            0xff6a6a
        };
        f.fill_rect(x0 + tx * k - 1, y0 + ty * k - 1, k + 2, k + 2, c);
    }
    // The window.
    let (wx, wy) = (x0 + cam_x * k / TILE, y0 + cam_y * k / TILE);
    let (ww, wh) = (f.w * k / TILE, f.h * k / TILE);
    for x in wx..wx + ww {
        f.blend(x, wy, 0xffffff, 160);
        f.blend(x, wy + wh - 1, 0xffffff, 160);
    }
    for y in wy..wy + wh {
        f.blend(wx, y, 0xffffff, 160);
        f.blend(wx + ww - 1, y, 0xffffff, 160);
    }
}

fn draw_banner(f: &mut Frame, x: i32, y: i32, color: u32) {
    let u = TILE / 24;
    let px = x + TILE / 2 - u;
    f.shade_disc(px + u, y + TILE - 3 * u, 4 * u, 0x000000, 60);
    f.fill_rect(px, y + 3 * u, 2 * u, TILE - 6 * u, 0x3b2a1a);
    f.fill_rect(px + 2 * u, y + 4 * u, 8 * u, 5 * u, color);
    f.fill_rect(px + 2 * u, y + 9 * u, 5 * u, 2 * u, color);
    f.fill_rect(px + 2 * u, y + 4 * u, 8 * u, u, arch::lighten(color, 0.35));
}

fn draw_bubble(f: &mut Frame, cx: i32, bottom: i32, text: &str) {
    let lines = wrap(text, 22, 3);
    if lines.is_empty() {
        return;
    }
    let w = lines.iter().map(|l| text_width(l, 2)).max().unwrap_or(0) + 14;
    let h = lines.len() as i32 * 16 + 10;
    let x = (cx - w / 2).clamp(2, f.w - w - 2);
    let y = (bottom - h - 6).max(2);
    f.shade_rect(x + 2, y + 2, w, h, 0x000000, 90);
    f.fill_rect(x, y, w, h, 0xf6f1e4);
    f.fill_rect(x + 2, y + 2, w - 4, h - 4, 0xfffdf6);
    // The tail toward the speaker.
    f.fill_rect(cx - 4, y + h, 8, 2, 0xf6f1e4);
    f.fill_rect(cx - 2, y + h + 2, 4, 2, 0xf6f1e4);
    f.fill_rect(cx - 1, y + h + 4, 2, 2, 0xf6f1e4);
    for (i, line) in lines.iter().enumerate() {
        f.text(x + 7, y + 5 + i as i32 * 16, line, 0x1b1f26, 2);
    }
}

fn draw_tile(f: &mut Frame, s: &Scene, tx: i32, ty: i32, x0: i32, y0: i32, ms: f64) {
    if x0 + TILE < 0 || y0 + TILE < 0 || x0 >= f.w || y0 >= f.h {
        return;
    }
    let tile = s.tile(tx, ty);
    let off_map = tx < 0 || ty < 0 || tx >= s.w || ty >= s.h;
    f.fill_rect(
        x0,
        y0,
        TILE,
        TILE,
        if off_map { 0x1f4a78 } else { tile.base() },
    );
    match tile {
        Tile::Grass | Tile::Forest => {
            for yy in 0..TILE {
                for xx in 0..TILE {
                    let h = hash(x0 + xx, y0 + yy, 1);
                    if h % 9 == 0 {
                        f.put(
                            x0 + xx,
                            y0 + yy,
                            if tile == Tile::Grass {
                                0x467d38
                            } else {
                                0x366a2d
                            },
                        );
                    } else if h % 23 == 0 {
                        f.put(x0 + xx, y0 + yy, 0x5c9a49);
                        f.put(x0 + xx, y0 + yy - 1, 0x5c9a49);
                    }
                }
            }
            // Shoreline: sand where grass meets water.
            for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                if s.tile(tx + dx, ty + dy) == Tile::Water {
                    let (rx, ry, rw, rh) = match (dx, dy) {
                        (0, -1) => (x0, y0, TILE, 6),
                        (0, 1) => (x0, y0 + TILE - 6, TILE, 6),
                        (-1, 0) => (x0, y0, 6, TILE),
                        _ => (x0 + TILE - 6, y0, 6, TILE),
                    };
                    f.shade_rect(rx, ry, rw, rh, 0xc9b58a, 120);
                }
            }
            if tile == Tile::Forest {
                let h = hash(tx, ty, 7);
                let cx = x0 + 14 + (h % 21) as i32;
                let cy = y0 + 16 + ((h >> 4) % 13) as i32;
                let r = 11 + ((h >> 8) % 4) as i32;
                f.shade_disc(cx + 4, cy + 6, r, 0x000000, 60);
                f.fill_rect(cx - 2, cy + r - 4, 5, 10, 0x5a3a1e);
                f.disc(cx, cy, r, 0x2f6a2b);
                f.disc(cx - 3, cy - 3, r - 5, 0x3f8a36);
                f.disc(cx - 5, cy - 6, (r - 8).max(2), 0x4f9a42);
            }
        }
        Tile::Water => {
            let drift = (ms / 140.0) as i32;
            let deep = off_map;
            for yy in 0..TILE {
                let py = y0 + yy;
                for xx in 0..TILE {
                    let px = x0 + xx;
                    let wave = (px + py * 3 + drift + (hash(0, py / 4, 3) % 5) as i32) % 17;
                    if wave == 0 {
                        f.put(px, py, if deep { 0x2a5a8a } else { 0x4d8fca });
                    } else if wave == 1 {
                        f.put(px, py, if deep { 0x24507a } else { 0x3d7ab8 });
                    } else if hash(px, py, 4) % 41 == 0 {
                        f.put(px, py, if deep { 0x1a3f66 } else { 0x245b91 });
                    }
                }
            }
        }
        Tile::Hill => {
            for yy in 0..TILE {
                for xx in 0..TILE {
                    let h = hash(x0 + xx, y0 + yy, 5);
                    if h % 11 == 0 {
                        f.put(x0 + xx, y0 + yy, 0x6f6553);
                    } else if h % 29 == 0 {
                        f.put(x0 + xx, y0 + yy, 0xa89c85);
                    }
                }
            }
            let h = hash(tx, ty, 9);
            let bx = x0 + 12 + (h % 20) as i32;
            let by = y0 + 18 + ((h >> 5) % 12) as i32;
            f.shade_disc(bx + 2, by + 4, 9, 0x000000, 50);
            f.disc(bx, by, 9, 0x7d7261);
            f.disc(bx - 2, by - 3, 6, 0x9d917d);
            f.disc(bx - 4, by - 5, 3, 0xb5a994);
        }
        Tile::Road => {
            for yy in 0..TILE {
                for xx in 0..TILE {
                    if hash(x0 + xx, y0 + yy, 6) % 13 == 0 {
                        f.put(x0 + xx, y0 + yy, 0xa08b64);
                    }
                }
            }
        }
        Tile::Town => {
            for yy in 0..TILE {
                for xx in 0..TILE {
                    let px = x0 + xx;
                    let py = y0 + yy;
                    if (tx * TILE + xx) % 12 == 0 || (ty * TILE + yy) % 12 == 0 {
                        f.put(px, py, 0x7f7c76);
                    } else if hash(px, py, 8) % 17 == 0 {
                        f.put(px, py, 0xaaa79f);
                    }
                }
            }
            // Cottages on the outer ring of the square, chosen by hash.
            let ring = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|(dx, dy)| s.tile(tx + dx, ty + dy) != Tile::Town);
            if ring && hash(tx, ty, 10) % 3 != 0 {
                let style = ["", "", "red", "blue", "timber"][(hash(tx, ty, 11) % 5) as usize];
                arch::draw_structure(
                    f,
                    (x0 + 4, y0 + 16, TILE - 8, TILE - 20),
                    "hut",
                    if style.is_empty() { None } else { Some(style) },
                    true,
                    1.0,
                    ms,
                );
            }
        }
    }
}

/// One character. Returns the top of the head, for the name and the bubble.
fn draw_figure(f: &mut Frame, fig: &Figure, px: i32, py: i32, moving: bool, phase: f32) -> i32 {
    let u = TILE / 24;
    let cx = px + TILE / 2;
    let feet = py + TILE - 3 * u;
    let bob = if moving && ((phase * 6.0) as i32) % 2 == 0 {
        -u
    } else {
        0
    };
    let color = if fig.npc { 0x6b5b95 } else { outfit(&fig.name) };
    f.shade_disc(cx, feet, 5 * u, 0x000000, 70);
    if fig.me {
        f.ring(cx, feet, 7 * u, 0xf2e28a);
        f.ring(cx, feet, 7 * u - 1, 0xf2e28a);
    }
    let top = feet - 13 * u + bob;
    // Legs
    let stride = if moving {
        (((phase * 6.0) as i32 % 2) * 2 - 1) * u
    } else {
        0
    };
    f.fill_rect(cx - 3 * u, top + 9 * u, 2 * u, 4 * u - bob, 0x2b2b33);
    f.fill_rect(cx + u + stride, top + 9 * u, 2 * u, 4 * u - bob, 0x2b2b33);
    // Body
    f.fill_rect(cx - 3 * u, top + 4 * u, 7 * u, 6 * u, color);
    f.fill_rect(cx - 3 * u, top + 4 * u, 7 * u, u, brighten(color));
    f.fill_rect(cx + 3 * u, top + 4 * u, u, 6 * u, darken(color));
    // Head
    f.disc(
        cx,
        top + u,
        3 * u,
        if fig.npc { 0xd8b48a } else { 0xf1c9a5 },
    );
    if fig.npc {
        // A hood.
        f.fill_rect(cx - 3 * u, top - 3 * u, 7 * u, 3 * u, 0x4a3f6b);
        f.fill_rect(cx - 4 * u, top - u, 9 * u, u, 0x4a3f6b);
    } else {
        f.fill_rect(cx - 3 * u, top - 2 * u, 7 * u, 2 * u, darken(color));
    }
    // A tool, swinging, when working.
    if fig.doing == "gather" || fig.doing == "build" {
        let swing = (phase * 4.0).sin();
        let hx = cx + 4 * u;
        let hy = top + 6 * u;
        let (tx, ty) = (
            hx + ((5.0 + swing * 2.0) * u as f32) as i32,
            hy - (4.0 * swing * u as f32) as i32 - 2 * u,
        );
        for i in 0..u {
            f.line(hx + i, hy, tx + i, ty, 0x5a3a1e);
        }
        let head = match (fig.doing.as_str(), fig.resource.as_deref()) {
            ("build", _) => 0x555a66,
            (_, Some("fish")) => 0x9ec7e8,
            (_, Some("wood")) => 0xa9a9a9,
            _ => 0x7a7a7a,
        };
        f.fill_rect(tx - u, ty - u, 3 * u, 3 * u, head);
    }
    top - 3 * u
}

fn brighten(c: u32) -> u32 {
    let ch = |v: u32| (v + (255 - v) / 3).min(255);
    ch(c >> 16 & 255) << 16 | ch(c >> 8 & 255) << 8 | ch(c & 255)
}

fn darken(c: u32) -> u32 {
    let ch = |v: u32| v * 2 / 3;
    ch(c >> 16 & 255) << 16 | ch(c >> 8 & 255) << 8 | ch(c & 255)
}

// ---------------------------------------------------------------------------
// The bitmap face: 5×7, uppercase, digits, a little punctuation
// ---------------------------------------------------------------------------

fn glyph(c: char) -> Option<[&'static str; 7]> {
    Some(match c.to_ascii_uppercase() {
        'A' => [
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'B' => [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
        'C' => [
            ".####", "#....", "#....", "#....", "#....", "#....", ".####",
        ],
        'D' => [
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        'F' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ],
        'G' => [
            ".####", "#....", "#....", "#.###", "#...#", "#...#", ".####",
        ],
        'H' => [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'I' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
        ],
        'J' => [
            "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ],
        'K' => [
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        'M' => [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
        'N' => [
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
        'O' => [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'P' => [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
        'Q' => [
            ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
        ],
        'R' => [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
        'S' => [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
        'T' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        'U' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'V' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..",
        ],
        'W' => [
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
        'X' => [
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
        'Y' => [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
        'Z' => [
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ],
        '0' => [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
        '1' => [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        '2' => [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
        '3' => [
            "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.",
        ],
        '4' => [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
        '5' => [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
        '6' => [
            "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.",
        ],
        '7' => [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
        '8' => [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
        '9' => [
            ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..",
        ],
        ' ' => [
            ".....", ".....", ".....", ".....", ".....", ".....", ".....",
        ],
        '.' => [
            ".....", ".....", ".....", ".....", ".....", ".....", "..#..",
        ],
        ',' => [
            ".....", ".....", ".....", ".....", ".....", "..#..", ".#...",
        ],
        '\'' => [
            "..#..", "..#..", ".....", ".....", ".....", ".....", ".....",
        ],
        '"' => [
            ".#.#.", ".#.#.", ".....", ".....", ".....", ".....", ".....",
        ],
        '-' => [
            ".....", ".....", ".....", ".###.", ".....", ".....", ".....",
        ],
        ':' => [
            ".....", ".....", "..#..", ".....", ".....", "..#..", ".....",
        ],
        ';' => [
            ".....", ".....", "..#..", ".....", ".....", "..#..", ".#...",
        ],
        '!' => [
            "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..",
        ],
        '?' => [
            ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..",
        ],
        '/' => [
            "....#", "...#.", "...#.", "..#..", ".#...", ".#...", "#....",
        ],
        '_' => [
            ".....", ".....", ".....", ".....", ".....", ".....", "#####",
        ],
        '(' => [
            "..#..", ".#...", ".#...", ".#...", ".#...", ".#...", "..#..",
        ],
        ')' => [
            "..#..", "...#.", "...#.", "...#.", "...#.", "...#.", "..#..",
        ],
        '…' => [
            ".....", ".....", ".....", ".....", ".....", ".....", "#.#.#",
        ],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_renders_through_the_camera_and_paints_every_pixel() {
        let mut w = world::World::new(7);
        let me = w.join("Ada");
        w.apply(
            me,
            &world::Command::CreateNpc {
                name: "Wren".into(),
                persona: "A forager who talks to birds.".into(),
            },
        )
        .unwrap();
        w.apply(
            me,
            &world::Command::Say {
                text: "hello there, what a fine morning to be alive in this world".into(),
            },
        )
        .unwrap();
        // A site waiting for stone, and a finished forge, both in the window.
        w.apply(
            me,
            &world::Command::FoundPlace {
                name: "Grey Spire".into(),
                description: "d".into(),
                resource: None,
                skill: None,
                form: world::Form::Spire,
                style: Some("dark".into()),
            },
        )
        .unwrap();
        w.apply(
            me,
            &world::Command::FoundPlace {
                name: "Anvil".into(),
                description: "d".into(),
                resource: None,
                skill: None,
                form: world::Form::Forge,
                style: None,
            },
        )
        .unwrap();
        if let Some(pl) = w.places.iter_mut().find(|p| p.name == "Anvil") {
            pl.needs.clear();
            pl.work = 100;
        }
        w.apply(
            me,
            &world::Command::Gather {
                resource: "iron".into(),
                amount: None,
            },
        )
        .unwrap();
        for _ in 0..12 {
            w.step();
        }
        let json = w.scene(Some(me));
        let scene = Scene::from_json(&json).expect("scene parses");
        assert_eq!(scene.figures.len(), 2);
        assert!(scene.figures[0].me);
        let spire = scene
            .places
            .iter()
            .find(|m| m.name == "Grey Spire")
            .unwrap();
        assert!(!spire.built() && spire.form == "spire" && spire.w == 2);
        let mut f = Frame::new(VIEW, VIEW);
        draw(&mut f, None, &scene, 1.0, 1234.0);
        assert!(
            f.px.chunks(4).all(|p| p[3] == 255),
            "every pixel is painted"
        );
        if let Ok(path) = std::env::var("CQS_SNAP") {
            std::fs::write(path, &f.px).unwrap();
        }
        // The camera keeps Ada in the window (in the middle unless the map
        // edge clamps it), and a spectator's window holds Town.
        let (cx, cy) = camera(&f, None, &scene, 1.0);
        let ada = &scene.figures[0];
        let (sx, sy) = (ada.x * TILE - cx, ada.y * TILE - cy);
        assert!(sx >= 0 && sx < VIEW && sy >= 0 && sy < VIEW, "{sx},{sy}");
        let spectator = Scene::from_json(&w.scene(None)).unwrap();
        let (cx, cy) = camera(&f, None, &spectator, 1.0);
        let town = spectator.places.iter().find(|m| m.name == "Town").unwrap();
        let (sx, sy) = (town.x * TILE - cx, town.y * TILE - cy);
        assert!(sx >= 0 && sx < VIEW && sy >= 0 && sy < VIEW, "{sx},{sy}");
        // Every form draws, finished and rising, without panicking.
        for form in [
            "hut", "house", "hall", "tower", "spire", "forge", "mill", "shrine", "well",
        ] {
            for (built, progress) in [(true, 1.0), (false, 0.0), (false, 0.5)] {
                arch::draw_structure(
                    &mut f,
                    (100, 100, 96, 96),
                    form,
                    Some("stone"),
                    built,
                    progress,
                    500.0,
                );
            }
        }
    }

    /// Every form, finished and rising, laid out on grass: for looking at.
    /// `CQS_GALLERY=path cargo test -p web gallery` writes the raw frame.
    #[test]
    fn gallery() {
        let Ok(path) = std::env::var("CQS_GALLERY") else {
            return;
        };
        let mut f = Frame::new(VIEW, VIEW);
        f.fill_rect(0, 0, VIEW, VIEW, 0x4f8a3f);
        let forms = [
            "hut", "house", "hall", "tower", "spire", "forge", "mill", "shrine", "well",
        ];
        let styles = [None, Some("stone"), Some("dark"), Some("purple")];
        for (row, style) in styles.iter().enumerate() {
            let y = 150 + row as i32 * 180;
            let mut x = 20;
            for form in forms {
                let w = if form == "hall" {
                    3
                } else if matches!(form, "hut" | "shrine" | "well") {
                    1
                } else {
                    2
                };
                let h = if matches!(form, "hut" | "shrine" | "well") {
                    1
                } else {
                    2
                };
                let (built, progress) = if row == 3 { (false, 0.55) } else { (true, 1.0) };
                let top = arch::draw_structure(
                    &mut f,
                    (x, y - h * 36, w * 36, h * 36),
                    form,
                    *style,
                    built,
                    progress,
                    700.0,
                );
                let label = form.to_uppercase();
                f.label(x, top - 12, &label, 0xf3f0e6, 1);
                x += w * 36 + 14;
            }
        }
        std::fs::write(path, &f.px).unwrap();
    }
}
