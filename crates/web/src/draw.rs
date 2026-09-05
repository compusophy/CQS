//! The display: a software framebuffer the scene is rasterized into, and the
//! rasterizers that do it. No canvas drawing API, no fonts from the browser —
//! rects, discs, lines, dither and a bitmap face, exactly as Tiny Empires
//! draws. The browser only ever receives finished pixels.

use gemini::Value;

/// One map tile is this many pixels; 32×18 tiles is 768×432, a 16:9 frame.
pub const TILE: i32 = 24;

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
}

#[derive(Clone, Debug, Default)]
pub struct Mark {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub resource: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Figure {
    pub name: String,
    pub x: i32,
    pub y: i32,
    /// "idle", "walk", "gather"
    pub doing: String,
    pub resource: Option<String>,
    pub me: bool,
    pub npc: bool,
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
            })
            .collect();
        figures.extend(v.get("npcs").as_arr().iter().map(|n| Figure {
            name: n.get("name").to_text(),
            x: n.get("x").as_i64().unwrap_or(0) as i32,
            y: n.get("y").as_i64().unwrap_or(0) as i32,
            doing: "idle".into(),
            resource: None,
            me: false,
            npc: true,
        }));
        Some(Scene {
            w,
            h,
            tick: v.get("tick").as_f64().unwrap_or(0.0) as u64,
            tiles,
            places,
            figures,
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
}

impl Frame {
    pub fn new(w: i32, h: i32) -> Frame {
        Frame {
            w,
            h,
            px: vec![0; (w * h * 4) as usize],
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
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
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
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
        for yy in y.max(0)..(y + h).min(self.h) {
            for xx in x.max(0)..(x + w).min(self.w) {
                self.put(xx, yy, rgb);
            }
        }
    }

    pub fn shade_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32, alpha: u32) {
        for yy in y.max(0)..(y + h).min(self.h) {
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

    /// Text with a one-pixel dark shadow, so it reads on any ground.
    pub fn label(&mut self, x: i32, y: i32, s: &str, rgb: u32, scale: i32) {
        self.text(x + scale, y + scale, s, 0x101418, scale);
        self.text(x, y, s, rgb, scale);
    }
}

pub fn text_width(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * 6 * scale
}

/// A small, fast, deterministic hash: the same speckle every frame.
fn hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

fn name_hash(s: &str) -> u32 {
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

/// Where a figure stands, in pixels, interpolated between two scenes.
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

/// Render one frame. `t` is how far (0..1) we are between `prev` and `cur`;
/// `ms` is a clock for animation.
pub fn draw(f: &mut Frame, prev: Option<&Scene>, cur: &Scene, t: f32, ms: f64) {
    let phase = (ms / 1000.0) as f32;
    // Ground
    for ty in 0..cur.h {
        for tx in 0..cur.w {
            draw_tile(f, cur, tx, ty, ms);
        }
    }
    // Places: a banner and a name
    for p in &cur.places {
        let x = p.x * TILE;
        let y = p.y * TILE;
        let color = p
            .resource
            .as_deref()
            .map(resource_color)
            .unwrap_or(0xd94a4a);
        if p.name == "Town" {
            draw_hall(f, x, y);
        }
        f.fill_rect(x + 11, y + 2, 2, 16, 0x3b2a1a);
        f.fill_rect(x + 13, y + 3, 8, 5, color);
        f.fill_rect(x + 13, y + 8, 5, 2, color);
        let label = p.name.to_uppercase();
        let w = text_width(&label, 1);
        f.label(x + 12 - w / 2, y - 7, &label, 0xf3f0e6, 1);
    }
    // Figures, back to front so nearer ones overlap
    let mut order: Vec<usize> = (0..cur.figures.len()).collect();
    order.sort_by_key(|&i| cur.figures[i].y);
    for i in order {
        let fig = &cur.figures[i];
        let before = prev.and_then(|s| {
            s.figures
                .iter()
                .find(|p| p.name == fig.name && p.npc == fig.npc)
        });
        let (px, py) = lerp_pos(before, fig, t);
        let moving = before
            .map(|b| (b.x, b.y) != (fig.x, fig.y))
            .unwrap_or(false)
            && fig.doing == "walk";
        draw_figure(f, fig, px, py, moving, phase);
    }
    // Vignette so the edges of the world read as edges
    for x in 0..f.w {
        f.blend(x, 0, 0x000000, 90);
        f.blend(x, f.h - 1, 0x000000, 90);
    }
    for y in 0..f.h {
        f.blend(0, y, 0x000000, 90);
        f.blend(f.w - 1, y, 0x000000, 90);
    }
}

fn draw_tile(f: &mut Frame, s: &Scene, tx: i32, ty: i32, ms: f64) {
    let x0 = tx * TILE;
    let y0 = ty * TILE;
    let tile = s.tile(tx, ty);
    let base = match tile {
        Tile::Grass => 0x4f8a3f,
        Tile::Water => 0x2c68a3,
        Tile::Forest => 0x3e7534,
        Tile::Hill => 0x8c8069,
        Tile::Road => 0xb8a27b,
        Tile::Town => 0x9a9791,
    };
    f.fill_rect(x0, y0, TILE, TILE, base);
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
            // Shoreline: darken grass next to water.
            for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                if s.tile(tx + dx, ty + dy) == Tile::Water {
                    let (rx, ry, rw, rh) = match (dx, dy) {
                        (0, -1) => (x0, y0, TILE, 3),
                        (0, 1) => (x0, y0 + TILE - 3, TILE, 3),
                        (-1, 0) => (x0, y0, 3, TILE),
                        _ => (x0 + TILE - 3, y0, 3, TILE),
                    };
                    f.shade_rect(rx, ry, rw, rh, 0xc9b58a, 120);
                }
            }
            if tile == Tile::Forest {
                let h = hash(tx, ty, 7);
                let cx = x0 + 8 + (h % 9) as i32;
                let cy = y0 + 9 + (h >> 4 % 7) as i32 % 6;
                let r = 6 + (h >> 8) as i32 % 3;
                f.shade_disc(cx + 2, cy + 3, r, 0x000000, 60);
                f.fill_rect(cx - 1, cy + r - 2, 3, 5, 0x5a3a1e);
                f.disc(cx, cy, r, 0x2f6a2b);
                f.disc(cx - 2, cy - 2, r - 3, 0x3f8a36);
            }
        }
        Tile::Water => {
            let drift = (ms / 140.0) as i32;
            for yy in 0..TILE {
                let py = y0 + yy;
                for xx in 0..TILE {
                    let px = x0 + xx;
                    let wave = (px + py * 3 + drift + (hash(0, py / 4, 3) % 5) as i32) % 17;
                    if wave == 0 {
                        f.put(px, py, 0x4d8fca);
                    } else if wave == 1 {
                        f.put(px, py, 0x3d7ab8);
                    } else if hash(px, py, 4) % 41 == 0 {
                        f.put(px, py, 0x245b91);
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
            let bx = x0 + 6 + (h % 8) as i32;
            let by = y0 + 10 + (h >> 5) as i32 % 6;
            f.shade_disc(bx + 1, by + 2, 5, 0x000000, 50);
            f.disc(bx, by, 5, 0x7d7261);
            f.disc(bx - 1, by - 2, 3, 0x9d917d);
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
                    if px % 6 == 0 || py % 6 == 0 {
                        f.put(px, py, 0x7f7c76);
                    } else if hash(px, py, 8) % 17 == 0 {
                        f.put(px, py, 0xaaa79f);
                    }
                }
            }
            // Houses on the outer ring of town tiles, chosen by hash.
            let ring = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|(dx, dy)| s.tile(tx + dx, ty + dy) != Tile::Town);
            if ring && hash(tx, ty, 10) % 3 != 0 {
                draw_house(f, x0 + 4, y0 + 5, hash(tx, ty, 11));
            }
        }
    }
}

fn draw_house(f: &mut Frame, x: i32, y: i32, h: u32) {
    let wall = [0xcbb89a, 0xd8c4a4, 0xbfa985][(h % 3) as usize];
    let roof = [0x8a3a2a, 0x6d4a2b, 0x5a4a6a][(h >> 3 % 3) as usize % 3];
    f.shade_rect(x + 2, y + 4, 16, 12, 0x000000, 50);
    f.fill_rect(x, y + 5, 16, 9, wall);
    for i in 0..5 {
        f.fill_rect(x - 1 + i, y + 5 - i, 18 - 2 * i, 1, roof);
    }
    f.fill_rect(x + 6, y + 9, 4, 5, 0x4a3320);
    f.fill_rect(x + 12, y + 8, 2, 2, 0xf3e6a0);
}

fn draw_hall(f: &mut Frame, x: i32, y: i32) {
    // The town hall behind the banner: wider, a lantern either side.
    f.shade_rect(x - 8, y + 6, 40, 16, 0x000000, 60);
    f.fill_rect(x - 10, y + 6, 40, 14, 0xd6c3a2);
    for i in 0..6 {
        f.fill_rect(x - 11 + i, y + 6 - i, 42 - 2 * i, 1, 0x7a3a2a);
    }
    f.fill_rect(x + 7, y + 12, 6, 8, 0x4a3320);
    f.fill_rect(x - 6, y + 10, 3, 3, 0xf3e6a0);
    f.fill_rect(x + 23, y + 10, 3, 3, 0xf3e6a0);
}

fn draw_figure(f: &mut Frame, fig: &Figure, px: i32, py: i32, moving: bool, phase: f32) {
    // Stand in the middle of the tile, feet near the bottom.
    let cx = px + TILE / 2;
    let feet = py + TILE - 3;
    let bob = if moving && ((phase * 6.0) as i32) % 2 == 0 {
        -1
    } else {
        0
    };
    let color = if fig.npc { 0x6b5b95 } else { outfit(&fig.name) };
    f.shade_disc(cx, feet, 5, 0x000000, 70);
    if fig.me {
        f.ring(cx, feet, 7, 0xf2e28a);
    }
    let top = feet - 13 + bob;
    // Legs
    let stride = if moving {
        ((phase * 6.0) as i32 % 2) * 2 - 1
    } else {
        0
    };
    f.fill_rect(cx - 3, top + 9, 2, 4 - bob, 0x2b2b33);
    f.fill_rect(cx + 1 + stride, top + 9, 2, 4 - bob, 0x2b2b33);
    // Body
    f.fill_rect(cx - 3, top + 4, 7, 6, color);
    f.fill_rect(cx - 3, top + 4, 7, 1, brighten(color));
    // Head
    f.disc(cx, top + 1, 3, if fig.npc { 0xd8b48a } else { 0xf1c9a5 });
    if fig.npc {
        // A hood.
        f.fill_rect(cx - 3, top - 3, 7, 3, 0x4a3f6b);
        f.fill_rect(cx - 4, top - 1, 9, 1, 0x4a3f6b);
    } else {
        f.fill_rect(cx - 3, top - 2, 7, 2, darken(color));
    }
    // A tool, swinging, when gathering.
    if fig.doing == "gather" {
        let swing = (phase * 4.0).sin();
        let hx = cx + 4;
        let hy = top + 6;
        let (tx, ty) = (
            hx + (5.0 + swing * 2.0) as i32,
            hy - (4.0 * swing) as i32 - 2,
        );
        f.line(hx, hy, tx, ty, 0x5a3a1e);
        let head = match fig.resource.as_deref() {
            Some("fish") => 0x9ec7e8,
            Some("wood") => 0xa9a9a9,
            _ => 0x7a7a7a,
        };
        f.fill_rect(tx - 1, ty - 1, 3, 3, head);
    }
    // Name
    let label = fig.name.to_uppercase();
    let w = text_width(&label, 1);
    let ink = if fig.me {
        0xfff2a8
    } else if fig.npc {
        0xd9ccff
    } else {
        0xf3f0e6
    };
    f.label(cx - w / 2, top - 12, &label, ink, 1);
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
        '-' => [
            ".....", ".....", ".....", ".###.", ".....", ".....", ".....",
        ],
        ':' => [
            ".....", ".....", "..#..", ".....", ".....", "..#..", ".....",
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
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_renders_without_panicking_and_paints_every_pixel() {
        let mut w = world::World::new(7);
        let me = w.join("Kyle");
        w.apply(
            me,
            &world::Command::Gather {
                resource: "iron".into(),
                amount: None,
            },
        )
        .unwrap();
        for _ in 0..40 {
            w.step();
        }
        let json = w.scene(Some(me));
        let scene = Scene::from_json(&json).expect("scene parses");
        assert_eq!(scene.figures.len(), 1);
        assert!(scene.figures[0].me);
        let mut f = Frame::new(scene.w * TILE, scene.h * TILE);
        draw(&mut f, None, &scene, 1.0, 1234.0);
        assert!(f.px.chunks(4).all(|p| p[3] == 255));
        let mut prev = scene.clone();
        prev.figures[0].x -= 1;
        draw(&mut f, Some(&prev), &scene, 0.5, 5678.0);
        assert_eq!(text_width("KYLE", 1), 24);
    }
}
