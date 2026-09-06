//! Buildings, at pixel resolution: the forms a place can take, drawn the way
//! Tiny Empires draws them. Walls with courses on a stone plinth, a shingled
//! roof with a sunlit pitch and a shaded one, a lit doorway — and for each
//! kind, a mark OUTSIDE the shell (a shaft, a stack, sails, a yard), because
//! two buildings that share a silhouette are one building at the size a
//! player actually reads the field from.
//!
//! Everything is in `u`, one plank: a twelfth of a tile, so the same drawing
//! scales from a 48 px hut to a 144 px hall. A site that is not yet built is
//! drawn rising out of the ground as its work is done, behind a scaffold.

use crate::draw::{hash, name_hash, Frame};

/// One civilisation's materials, chosen by the founder's style word.
#[derive(Clone, Copy)]
pub struct Arch {
    pub wall: u32,
    pub wall_lit: u32,
    pub wall_dim: u32,
    pub course: u32,
    pub plinth: (u32, u32),
    pub roof: u32,
    pub roof_lit: u32,
    pub roof_dim: u32,
    pub door: (u32, u32),
    pub flag: (u32, u32),
    pub glow: u32,
    /// Their vocabulary: iron teeth on the ridge and horns at the eaves.
    pub grim: bool,
}

pub fn arch_of(style: Option<&str>) -> Arch {
    let base = Arch {
        wall: 0xd8c49c,
        wall_lit: 0xeeddb8,
        wall_dim: 0xa88f68,
        course: 0xc4ac82,
        plinth: (0x8d8579, 0xa9a294),
        roof: 0xa8563a,
        roof_lit: 0xc87a52,
        roof_dim: 0x6e3524,
        door: (0x2a1d12, 0x6b4a30),
        flag: (0xd94a4a, 0x4a3a28),
        glow: 0xffc55e,
        grim: false,
    };
    let s = style.map(|s| s.trim().to_ascii_lowercase());
    match s.as_deref() {
        None | Some("") | Some("daub") | Some("plaster") | Some("cream") => base,
        Some("stone") | Some("grey") | Some("gray") | Some("granite") | Some("slate") => Arch {
            wall: 0xa9a294,
            wall_lit: 0xc4bfb2,
            wall_dim: 0x7d776b,
            course: 0x958f82,
            roof: 0x5a6270,
            roof_lit: 0x7a8494,
            roof_dim: 0x3a404a,
            ..base
        },
        Some("dark") | Some("black") | Some("shadow") | Some("iron") | Some("obsidian")
        | Some("grim") | Some("cursed") | Some("evil") => Arch {
            wall: 0x5b4b40,
            wall_lit: 0x776357,
            wall_dim: 0x3a2f28,
            course: 0x4a3d34,
            plinth: (0x3f3a3c, 0x565054),
            roof: 0x33303a,
            roof_lit: 0x4e4b58,
            roof_dim: 0x1c1a22,
            door: (0x14100e, 0x2a2420),
            flag: (0x8a2030, 0x2a2420),
            glow: 0xff7a30,
            grim: true,
        },
        Some("white") | Some("marble") | Some("ivory") | Some("pale") | Some("silver") => Arch {
            wall: 0xf0ece0,
            wall_lit: 0xfffdf4,
            wall_dim: 0xc9c3b3,
            course: 0xe2ddcf,
            roof: 0x7d8ea8,
            roof_lit: 0x9fb0c8,
            roof_dim: 0x55627a,
            ..base
        },
        Some("red") | Some("crimson") | Some("scarlet") => Arch {
            roof: 0xb83a2a,
            roof_lit: 0xd85a42,
            roof_dim: 0x7a2418,
            flag: (0xd92a2a, 0x4a3a28),
            ..base
        },
        Some("blue") | Some("azure") | Some("sea") => Arch {
            roof: 0x3a5fa8,
            roof_lit: 0x5a7fc8,
            roof_dim: 0x243c70,
            flag: (0x3b7dd8, 0x4a3a28),
            ..base
        },
        Some("gold") | Some("golden") | Some("yellow") | Some("brass") => Arch {
            roof: 0xd9a63a,
            roof_lit: 0xf0c45a,
            roof_dim: 0x8a6a1e,
            flag: (0xf2c94c, 0x4a3a28),
            glow: 0xfff0a0,
            ..base
        },
        Some("green") | Some("mossy") | Some("moss") | Some("ivy") | Some("forest") => Arch {
            wall: 0xb8b894,
            wall_lit: 0xd0d0aa,
            wall_dim: 0x8a8a68,
            course: 0xa6a684,
            roof: 0x4f7a3a,
            roof_lit: 0x6a9a50,
            roof_dim: 0x2f4a24,
            flag: (0x2fa36b, 0x4a3a28),
            ..base
        },
        Some("purple") | Some("violet") | Some("crystal") | Some("arcane") | Some("magic")
        | Some("amethyst") => Arch {
            wall: 0xb9a7d8,
            wall_lit: 0xd6c8ee,
            wall_dim: 0x8a78ac,
            course: 0xa998c8,
            roof: 0x6b4fa8,
            roof_lit: 0x8e72cc,
            roof_dim: 0x40306a,
            flag: (0x8e5bd6, 0x4a3a28),
            glow: 0xc9a6ff,
            ..base
        },
        Some("timber") | Some("wood") | Some("wooden") | Some("oak") | Some("log")
        | Some("pine") => Arch {
            wall: 0x9a7048,
            wall_lit: 0xb98a5c,
            wall_dim: 0x6b4a30,
            course: 0x85603e,
            roof: 0x6d4a2b,
            roof_lit: 0x8a6240,
            roof_dim: 0x40291a,
            ..base
        },
        Some(other) => {
            // Any other word is a roof colour of its own, the same every time.
            const ROOFS: [(u32, u32, u32); 8] = [
                (0xa8563a, 0xc87a52, 0x6e3524),
                (0x3a5fa8, 0x5a7fc8, 0x243c70),
                (0x4f7a3a, 0x6a9a50, 0x2f4a24),
                (0x6b4fa8, 0x8e72cc, 0x40306a),
                (0xd9a63a, 0xf0c45a, 0x8a6a1e),
                (0x20a5a5, 0x40c5c5, 0x146a6a),
                (0xc94f9a, 0xe070b8, 0x80306a),
                (0x5a6270, 0x7a8494, 0x3a404a),
            ];
            let (roof, roof_lit, roof_dim) = ROOFS[(name_hash(other) % 8) as usize];
            Arch {
                roof,
                roof_lit,
                roof_dim,
                flag: (roof_lit, 0x4a3a28),
                ..base
            }
        }
    }
}

pub fn shade(c: u32, k: f32) -> u32 {
    let ch = |v: u32| ((v as f32 * k).round() as u32).min(255);
    ch(c >> 16 & 255) << 16 | ch(c >> 8 & 255) << 8 | ch(c & 255)
}

pub fn lighten(c: u32, k: f32) -> u32 {
    let ch = |v: u32| (v as f32 + (255.0 - v as f32) * k).round() as u32;
    ch(c >> 16 & 255) << 16 | ch(c >> 8 & 255) << 8 | ch(c & 255)
}

/// A rectangle by its corners, the way the Tiny Empires art is written.
fn r(f: &mut Frame, x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
    let (x, w) = (x0.min(x1), (x1 - x0).abs());
    let (y, h) = (y0.min(y1), (y1 - y0).abs());
    if w > 0 && h > 0 {
        f.fill_rect(x, y, w, h, c);
    }
}

fn thick_line(f: &mut Frame, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, c: u32) {
    for i in 0..t.max(1) {
        f.line(x0 + i, y0, x1 + i, y1, c);
    }
}

const SHADOW: u32 = 0x1a2a1a;

/// How far above its rect a form draws: where its label goes.
pub fn top_of(form: &str, (_, y, w, h): (i32, i32, i32, i32)) -> i32 {
    let u = (w / 24).max(1);
    match form {
        "tower" => y - 17 * u,
        "spire" => y - 40 * u,
        "hall" => y - h / 2 - 4 * u,
        "mill" => y - h - 2 * u,
        "forge" => y - h * 2 / 5 - 8 * u,
        "shrine" | "well" => y - h / 2 - 2 * u,
        "hut" => y - h * 2 / 5 - 2 * u,
        _ => y - h * 2 / 5 - 6 * u,
    }
}

/// One building, finished or rising, in PIXEL coordinates. `r` is the
/// footprint rect; the drawing stands on its bottom edge and rises above its
/// top. Returns the topmost row of ink, for the label.
#[allow(clippy::too_many_arguments)]
pub fn draw_structure(
    f: &mut Frame,
    rect: (i32, i32, i32, i32),
    form: &str,
    style: Option<&str>,
    built: bool,
    progress: f32,
    ms: f64,
) -> i32 {
    let a = arch_of(style);
    let top = top_of(form, rect);
    if built {
        finished(f, rect, form, &a, ms);
        return top;
    }
    let (x, y, w, h) = rect;
    let u = (w / 24).max(1);
    let foot = y + h;
    // The site: stakes at the corners with a rope between them, and the
    // ground turned over inside.
    for yy in y..foot {
        for xx in x..x + w {
            if hash(xx, yy, 21) % 5 == 0 {
                f.put(xx, yy, 0x6f5a3a);
            }
        }
    }
    for (sx, sy) in [(x, y), (x + w - u, y), (x, foot - u), (x + w - u, foot - u)] {
        r(f, sx, sy - 3 * u, sx + u, sy + u, 0x8a6a3a);
        r(f, sx, sy - 3 * u, sx + u, sy - 2 * u, 0xb08a4a);
    }
    for (x0, y0, x1, y1) in [
        (x, y - 2 * u, x + w, y - 2 * u),
        (x, foot - 3 * u, x + w, foot - 3 * u),
        (x, y - 2 * u, x, foot - 3 * u),
        (x + w - u, y - 2 * u, x + w - u, foot - 3 * u),
    ] {
        f.line(x0, y0, x1, y1, 0xc9b58a);
    }
    if progress <= 0.0 {
        // Waiting for materials: a signboard by the front.
        let (px, py) = (x + w / 2 - 3 * u, foot - u);
        r(f, px + 2 * u, py - 6 * u, px + 3 * u, py, 0x6b4a30);
        r(f, px, py - 8 * u, px + 6 * u, py - 5 * u, 0xd8c49c);
        r(f, px + u, py - 7 * u, px + 5 * u, py - 6 * u, 0x8a6a3a);
        return foot - 8 * u;
    }
    // Rising: the finished building, clipped to how much of it stands, behind
    // a scaffold that reaches the current top.
    let rise = foot - ((foot - top) as f32 * progress.clamp(0.0, 1.0)) as i32;
    f.clip_top = rise;
    finished(f, rect, form, &a, ms);
    f.clip_top = 0;
    let post = 0x9a7048;
    for sx in [x - u, x + w] {
        r(f, sx, rise - 2 * u, sx + u, foot, post);
    }
    let mut by = foot - 4 * u;
    while by > rise - 2 * u {
        r(
            f,
            x - u,
            by,
            x + w + u,
            by + (u / 2).max(1),
            lighten(post, 0.2),
        );
        by -= 4 * u;
    }
    r(
        f,
        x - u,
        rise - 2 * u,
        x + w + u,
        rise - 2 * u + u,
        lighten(post, 0.3),
    );
    rise - 3 * u
}

fn finished(f: &mut Frame, rect: (i32, i32, i32, i32), form: &str, a: &Arch, ms: f64) {
    match form {
        "hut" => hut(f, rect, a),
        "hall" => hall(f, rect, a),
        "tower" => tower(f, rect, a),
        "spire" => spire(f, rect, a, ms),
        "forge" => forge(f, rect, a, ms),
        "mill" => mill(f, rect, a, ms),
        "shrine" => shrine(f, rect, a, ms),
        "well" => well(f, rect, a),
        _ => {
            house(f, rect, a, true, true);
        }
    }
}

/// The shell every daub-and-timber building shares: walls, plinth, roof,
/// door, windows. Returns (wall_y, foot, eave, unit) for the marks on top.
fn house(
    f: &mut Frame,
    rect: (i32, i32, i32, i32),
    a: &Arch,
    chimney: bool,
    two_windows: bool,
) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = rect;
    let u = (w / 24).max(1);
    let eave = u * 2;
    let roof_h = h * 2 / 5;
    let wall_y = y + roof_h;
    let plinth_h = (h / 12).max(u);
    let foot = y + h;

    // Cast shadow, down and right: light comes from the upper left.
    f.shade_rect(x + eave, foot, w, plinth_h + u, SHADOW, 80);

    // Walls: a lit left band, a shaded right band, courses, and the footing.
    r(f, x + eave, wall_y, x + w - eave, foot, a.wall);
    r(f, x + eave, wall_y, x + eave + w / 4, foot, a.wall_lit);
    r(
        f,
        x + w - eave - w / 5,
        wall_y,
        x + w - eave,
        foot,
        a.wall_dim,
    );
    let mut cy = wall_y + u * 3;
    while cy < foot - plinth_h {
        r(f, x + eave, cy, x + w - eave, cy + (u / 2).max(1), a.course);
        cy += u * 4;
    }
    r(f, x + eave, foot - plinth_h, x + w - eave, foot, a.plinth.0);
    r(
        f,
        x + eave,
        foot - plinth_h,
        x + w - eave,
        foot - plinth_h + u,
        a.plinth.1,
    );

    // Roof: courses of shingles from the ridge down, each a step wider.
    let rows = (roof_h / u).max(3);
    let cx = x + w / 2;
    for i in 0..rows {
        let t = i as f32 / (rows - 1).max(1) as f32;
        let half = ((w as f32 / 2.0) * (0.10 + 0.90 * t)) as i32;
        let (ry0, ry1) = (y + i * roof_h / rows, y + (i + 1) * roof_h / rows);
        r(f, cx - half, ry0, cx, ry1, a.roof_lit);
        r(f, cx, ry0, cx + half, ry1, a.roof);
        r(f, cx + half - u, ry0, cx + half, ry1, a.roof_dim);
        let step = (u * 3).max(2);
        let mut jx = cx - half + if i % 2 == 0 { step / 2 } else { 0 };
        while jx < cx + half {
            r(
                f,
                jx,
                ry1 - (u / 2).max(1),
                jx + (u / 2).max(1),
                ry1,
                a.roof_dim,
            );
            jx += step;
        }
    }
    r(
        f,
        x + eave,
        wall_y,
        x + w - eave,
        wall_y + u,
        shade(a.wall, 0.55),
    );

    if a.grim {
        // Iron teeth the length of the ridge, and hooks off both eaves.
        let mut sx = x + eave;
        while sx < x + w - eave {
            let tall = if (sx - x - eave) / (u * 3) % 2 == 0 {
                u * 4
            } else {
                u * 2
            };
            r(f, sx, y - tall, sx + u, y + u, a.roof_lit);
            r(f, sx, y - tall, sx + u, y - tall + u, a.roof_dim);
            sx += u * 3;
        }
        for (hx, dir) in [(x, -1), (x + w, 1)] {
            let arm = eave * 2;
            let (x0, x1) = (hx.min(hx + dir * arm), hx.max(hx + dir * arm));
            r(f, x0, y + roof_h - u * 2, x1, y + roof_h, a.roof_dim);
            let tip = if dir < 0 { x0 } else { x1 - u * 2 };
            r(
                f,
                tip,
                y + roof_h - u * 2,
                tip + u * 2,
                y + roof_h + u * 5,
                a.roof_dim,
            );
        }
    } else if chimney {
        let chx = x + w / 4;
        r(
            f,
            chx,
            y - roof_h / 3,
            chx + u * 3,
            y + roof_h / 2,
            a.plinth.0,
        );
        r(
            f,
            chx,
            y - roof_h / 3,
            chx + u * 3,
            y - roof_h / 3 + u,
            a.plinth.1,
        );
    }

    // The door, with a timber lintel and a lit threshold.
    let (dw, dh) = ((w / 6).max(u * 3), (h / 4).max(u * 5));
    let dx = x + w / 2 - dw / 2;
    r(f, dx - u, foot - dh - u, dx + dw + u, foot, a.door.1);
    r(f, dx, foot - dh, dx + dw, foot, a.door.0);
    if !a.grim {
        r(f, dx, foot - u, dx + dw, foot, 0x7a5a34);
    }
    // Windows, lit: a house with a light in it is a house someone lives in.
    let (wy, wh) = (wall_y + u * 3, (h / 8).max(u * 2));
    let sides: &[i32] = if two_windows { &[-1, 1] } else { &[1] };
    for side in sides {
        let cx = x + w / 2 + side * (w / 4);
        if a.grim {
            r(
                f,
                cx - (u / 2).max(1),
                wy,
                cx + (u / 2).max(1),
                wy + wh * 2,
                0x241f1c,
            );
        } else {
            r(f, cx - u * 2, wy, cx + u * 2, wy + wh, a.door.1);
            r(
                f,
                cx - u,
                wy + (u / 2).max(1),
                cx + u,
                wy + wh - (u / 2).max(1),
                a.glow,
            );
        }
    }
    (wall_y, foot, eave, u)
}

/// A banner on a pole: the founder's colours.
#[allow(clippy::too_many_arguments)]
pub fn banner(
    f: &mut Frame,
    a: &Arch,
    pole: i32,
    top: i32,
    bottom: i32,
    bw: i32,
    rows: i32,
    u: i32,
) {
    r(f, pole, top, pole + u, bottom, a.flag.1);
    for k in 0..rows {
        let yy = top + k * u;
        let fly = if a.grim {
            bw - ((k * 5 + 2) % 4) * u / 2
        } else {
            bw - (rows / 2 - (k - rows / 2).abs()) * u / 2
        };
        let ink = if k == 0 {
            lighten(a.flag.0, 0.35)
        } else {
            a.flag.0
        };
        r(f, pole + u, yy, pole + u + fly.max(u), yy + u, ink);
    }
}

fn hut(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch) {
    // A cottage: the shell with a thatched roof unless the style says stone.
    let thatch = Arch {
        roof: 0xb89b52,
        roof_lit: 0xd1b56a,
        roof_dim: 0x7f6a34,
        ..*a
    };
    let a2 = if a.roof == arch_of(None).roof {
        &thatch
    } else {
        a
    };
    house(f, rect, a2, false, false);
}

fn hall(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch) {
    let (x, y, w, h) = rect;
    let (wall_y, foot, eave, u) = house(f, rect, a, true, true);
    let cx = x + w / 2;
    // The skirt roof: a second, flared course under the main eave, thrown
    // wide of the walls — what a hall has and a cottage does not.
    let sk_h = u * 3;
    for i in 0..3 {
        let t = i as f32 / 2.0;
        let half = (w / 2 - eave) + (eave as f32 * 3.0 * t) as i32;
        let (ry0, ry1) = (wall_y + i * sk_h / 3, wall_y + (i + 1) * sk_h / 3);
        r(f, cx - half, ry0, cx, ry1, a.roof_lit);
        r(f, cx, ry0, cx + half, ry1, a.roof);
        r(f, cx + half - u, ry0, cx + half, ry1, a.roof_dim);
    }
    r(
        f,
        x + eave,
        wall_y + sk_h,
        x + w - eave,
        wall_y + sk_h + u,
        shade(a.wall, 0.55),
    );
    // Corner posts carrying it past the wall face.
    for sgn in [-1, 1] {
        let px = if sgn < 0 {
            x + eave - u * 2
        } else {
            x + w - eave
        };
        r(f, px, wall_y + sk_h, px + u * 2, foot + u, a.door.1);
        r(
            f,
            px,
            wall_y + sk_h,
            px + u,
            foot + u,
            lighten(a.door.1, 0.3),
        );
    }
    let ph = h / 2;
    banner(
        f,
        a,
        x + w - eave - u,
        y - ph,
        wall_y,
        w / 4,
        ((ph / 2) / u).max(1),
        u,
    );
}

fn tower(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch) {
    let (x, y, w, h) = rect;
    let u = (w / 24).max(1);
    let foot = y + h;
    let cx = x + w / 2;
    let (stone, lit) = a.plinth;
    let dim = shade(stone, 0.6);
    f.shade_rect(x + 7 * u, foot, 14 * u, 2 * u, SHADOW, 80);
    // The shaft: courses are what sell the height.
    let (sx0, sx1) = (x + 7 * u, x + 17 * u);
    let top = y - 10 * u;
    r(f, sx0, top, sx1, foot, stone);
    r(f, sx0, top, sx0 + 3 * u, foot, lit);
    r(f, sx1 - 2 * u, top, sx1, foot, dim);
    let mut cy = top + 4 * u;
    while cy < foot - 4 * u {
        r(f, sx0, cy, sx1, cy + u, shade(stone, 0.8));
        cy += 5 * u;
    }
    r(f, x + 5 * u, foot - 4 * u, x + 19 * u, foot, stone);
    r(f, x + 5 * u, foot - 4 * u, x + 19 * u, foot - 3 * u, lit);
    // The fighting platform, oversailing the shaft, with its shadowed underside.
    r(f, x + 4 * u, y - 13 * u, x + 20 * u, y - 10 * u, stone);
    r(
        f,
        x + 4 * u,
        y - 11 * u,
        x + 20 * u,
        y - 10 * u,
        shade(stone, 0.55),
    );
    r(f, x + 4 * u, y - 13 * u, x + 20 * u, y - 12 * u, lit);
    for bx in [x + 5 * u, x + 17 * u] {
        r(f, bx, y - 10 * u, bx + 2 * u, y - 8 * u, shade(stone, 0.7));
    }
    if a.grim {
        let mut tx = x + 5 * u;
        while tx < x + 19 * u {
            r(f, tx, y - 17 * u, tx + u, y - 13 * u, a.roof_lit);
            r(f, tx, y - 17 * u, tx + u, y - 16 * u, a.roof_dim);
            tx += 3 * u;
        }
    } else {
        for mx in [x + 4 * u, x + 10 * u, x + 16 * u] {
            r(f, mx, y - 16 * u, mx + 4 * u, y - 13 * u, stone);
            r(f, mx, y - 16 * u, mx + 4 * u, y - 15 * u, lit);
        }
    }
    let slot = if a.grim { 0x241f1c } else { a.door.0 };
    for (sy0, sy1) in [(y - 8 * u, y - u), (y + 6 * u, y + 11 * u)] {
        r(f, cx - u, sy0, cx + u, sy1, slot);
        r(f, cx - u, sy1 - u, cx + u, sy1, shade(lit, 0.8));
    }
    // The door at the foot, and the pennant off the platform's corner.
    r(
        f,
        cx - 2 * u,
        foot - 6 * u,
        cx + 2 * u,
        foot - 4 * u,
        a.door.1,
    );
    r(
        f,
        cx - u - (u / 2),
        foot - 5 * u,
        cx + u + (u / 2),
        foot - 4 * u,
        a.door.0,
    );
    banner(f, a, x + 19 * u, y - 10 * u, y + 4 * u, 5 * u, 4, u);
}

fn spire(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch, ms: f64) {
    // A wizard's tower: a house at the foot, and a stone shaft standing proud
    // of its roof with the arcane light burning in a socket at the top.
    let (x, y, w, h) = rect;
    let u = (w / 24).max(1);
    let foot = y + h;
    let cx = x + w / 2;
    let plinth_h = (h / 12).max(u);
    let (_, _, _, _) = house(f, rect, a, false, true);
    let sw = w / 5;
    let top = y - 22 * u;
    let (stone, lit) = (0x8a8496, 0xaaa4b8);
    let dim = 0x625d70;
    r(f, cx - sw, top, cx + sw, foot - plinth_h, stone);
    r(f, cx - sw, top, cx - sw + u * 2, foot - plinth_h, lit);
    r(f, cx + sw - u * 2, top, cx + sw, foot - plinth_h, dim);
    let mut cy = top + 4 * u;
    while cy < y {
        r(
            f,
            cx - sw,
            cy,
            cx + sw,
            cy + (u / 2).max(1),
            shade(stone, 0.8),
        );
        cy += 4 * u;
    }
    // A slit window up the shaft, lit from within.
    r(f, cx - u, top + 6 * u, cx + u, top + 11 * u, a.door.0);
    r(
        f,
        cx - (u / 2).max(1),
        top + 7 * u,
        cx + (u / 2).max(1),
        top + 10 * u,
        a.glow,
    );
    // The conical cap, in the roof's colour, and the orb above it.
    let cap_h = 6 * u;
    for i in 0..cap_h {
        let half = (sw + u) * (i + 1) / cap_h;
        r(
            f,
            cx - half,
            top - cap_h + i,
            cx,
            top - cap_h + i + 1,
            a.roof_lit,
        );
        r(
            f,
            cx,
            top - cap_h + i,
            cx + half,
            top - cap_h + i + 1,
            a.roof,
        );
    }
    let pulse = ((ms / 600.0).sin() * 0.5 + 0.5) as f32;
    let oy = top - cap_h - sw;
    f.shade_disc(cx, oy, sw + 2 * u + (pulse * u as f32) as i32, a.glow, 60);
    f.disc(cx, oy, sw, a.glow);
    f.disc(cx - sw / 3, oy - sw / 3, (sw / 3).max(1), 0xfff0ff);
    // Motes drifting up from the orb.
    for k in 0..4 {
        let t = ((ms / 900.0) as f32 + k as f32 * 0.25) % 1.0;
        let mx = cx + ((k as f32 * 1.7 + t * 6.28).sin() * (sw as f32)) as i32;
        let my = oy - (t * 10.0 * u as f32) as i32;
        f.blend(mx, my, a.glow, (200.0 * (1.0 - t)) as u32);
    }
}

fn forge(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch, ms: f64) {
    let (x, y, w, h) = rect;
    let (_, foot, eave, u) = house(f, rect, a, false, false);
    let roof_h = h * 2 / 5;
    let plinth_h = (h / 12).max(u);
    // A stack up the side, smoking, and a furnace mouth that is alight.
    let sx = x + w - eave - w / 5;
    r(f, sx, y - roof_h, sx + w / 6, foot - plinth_h, 0x4a3a34);
    r(f, sx, y - roof_h, sx + u, foot - plinth_h, 0x67534c);
    for k in 0..3 {
        let t = ((ms / 1400.0) as f32 + k as f32 * 0.33) % 1.0;
        let px = sx + w / 12 + ((t * 9.0).sin() * 2.0 * u as f32) as i32;
        let py = y - roof_h - (t * 12.0 * u as f32) as i32;
        f.shade_disc(
            px,
            py,
            u + (t * 2.0 * u as f32) as i32,
            0x9a948c,
            (140.0 * (1.0 - t)) as u32,
        );
    }
    let dw = (w / 6).max(u * 3);
    let (mx, my) = (x + w / 3, foot - (h / 4).max(u * 5));
    let flicker = if (ms / 120.0) as i64 % 3 == 0 {
        0xffb040
    } else {
        0xff8a30
    };
    r(
        f,
        mx - dw / 2 - u,
        my - u,
        mx + dw / 2 + u,
        foot - plinth_h,
        0x3a2a22,
    );
    r(f, mx - dw / 2, my, mx + dw / 2, foot - plinth_h, flicker);
    // The anvil in the yard.
    let (ax, ay) = (x + w - eave - 3 * u, foot + 2 * u);
    r(f, ax - 3 * u, ay - 2 * u, ax + 3 * u, ay - u, 0x555a66);
    r(f, ax - u, ay - u, ax + u, ay + u, 0x3f434c);
    r(f, ax - 2 * u, ay + u, ax + 2 * u, ay + 2 * u, 0x3f434c);
}

fn mill(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch, ms: f64) {
    let (x, y, w, h) = rect;
    let (_, _, _, u) = house(f, rect, a, false, true);
    // Sails on a hub above the ridge, turning.
    let cx = x + w / 2;
    let hub_y = y - 4 * u;
    r(f, cx - u, hub_y, cx + u, y + h / 5, 0x6b4a30);
    let ang = (ms / 1800.0) as f32;
    let len = (h * 3 / 5) as f32;
    for k in 0..4 {
        let t = ang + k as f32 * std::f32::consts::FRAC_PI_2;
        let (dx, dy) = (t.cos() * len, t.sin() * len);
        let (tx, ty) = (cx + dx as i32, hub_y + dy as i32);
        thick_line(f, cx, hub_y, tx, ty, u, 0x8a6a3a);
        // The cloth on the leading half of each sail.
        let (nx, ny) = (-dy / len, dx / len);
        for s in 1..=(len as i32 / 2) {
            let (px, py) = (
                cx as f32 + dx * (0.35 + 0.65 * s as f32 / (len / 2.0)),
                hub_y as f32 + dy * (0.35 + 0.65 * s as f32 / (len / 2.0)),
            );
            let wdt = (3 * u) as f32;
            f.line(
                px as i32,
                py as i32,
                (px + nx * wdt) as i32,
                (py + ny * wdt) as i32,
                0xe8dcc0,
            );
        }
    }
    f.disc(cx, hub_y, u + u / 2, 0x4a3320);
}

fn shrine(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch, ms: f64) {
    let (x, y, w, h) = rect;
    let u = (w / 24).max(1);
    let foot = y + h;
    let (stone, lit) = a.plinth;
    let cx = x + w / 2;
    f.shade_rect(x + 3 * u, foot - u, w - 4 * u, 3 * u, SHADOW, 70);
    // Steps, two pillars, a small pitched roof, and a candle between them.
    r(f, x + 2 * u, foot - 3 * u, x + w - 2 * u, foot, stone);
    r(f, x + 2 * u, foot - 3 * u, x + w - 2 * u, foot - 2 * u, lit);
    r(
        f,
        x + 4 * u,
        foot - 5 * u,
        x + w - 4 * u,
        foot - 3 * u,
        stone,
    );
    for px in [x + 5 * u, x + w - 7 * u] {
        r(f, px, y - 2 * u, px + 2 * u, foot - 5 * u, lit);
        r(
            f,
            px + u,
            y - 2 * u,
            px + 2 * u,
            foot - 5 * u,
            shade(stone, 0.7),
        );
    }
    let roof_h = h / 2;
    for i in 0..roof_h {
        let half = (w / 2) * (i + 1) / roof_h;
        r(
            f,
            cx - half,
            y - roof_h + i,
            cx,
            y - roof_h + i + 1,
            a.roof_lit,
        );
        r(f, cx, y - roof_h + i, cx + half, y - roof_h + i + 1, a.roof);
    }
    let flame = if (ms / 150.0) as i64 % 2 == 0 {
        0xffe08a
    } else {
        0xffc040
    };
    r(f, cx - u, foot - 8 * u, cx + u, foot - 5 * u, 0xf0ead6);
    f.disc(cx, foot - 9 * u, u, flame);
    f.shade_disc(cx, foot - 8 * u, 3 * u, a.glow, 50);
}

fn well(f: &mut Frame, rect: (i32, i32, i32, i32), a: &Arch) {
    let (x, y, w, h) = rect;
    let u = (w / 24).max(1);
    let foot = y + h;
    let (stone, lit) = a.plinth;
    let cx = x + w / 2;
    f.shade_rect(x + 4 * u, foot - 2 * u, w - 6 * u, 3 * u, SHADOW, 70);
    // The ring of stones with water in it, two posts, and a little roof.
    let ry = foot - 6 * u;
    r(f, x + 4 * u, ry, x + w - 4 * u, foot - u, stone);
    r(f, x + 4 * u, ry, x + w - 4 * u, ry + u, lit);
    r(f, x + 6 * u, ry + u, x + w - 6 * u, ry + 3 * u, 0x2c68a3);
    r(f, x + 7 * u, ry + u, x + w - 9 * u, ry + 2 * u, 0x4d8fca);
    for px in [x + 4 * u, x + w - 5 * u] {
        r(f, px, y + 2 * u, px + u, ry, 0x6b4a30);
    }
    let roof_h = 5 * u;
    for i in 0..roof_h {
        let half = (w / 2 - 2 * u) * (i + 1) / roof_h;
        r(
            f,
            cx - half,
            y + 2 * u - roof_h + i,
            cx,
            y + 2 * u - roof_h + i + 1,
            a.roof_lit,
        );
        r(
            f,
            cx,
            y + 2 * u - roof_h + i,
            cx + half,
            y + 2 * u - roof_h + i + 1,
            a.roof,
        );
    }
    // The windlass and the bucket on its rope.
    r(f, x + 4 * u, y + 3 * u, x + w - 4 * u, y + 4 * u, 0x8a6a3a);
    f.line(cx, y + 4 * u, cx, ry - u, 0x3a2a1a);
    r(f, cx - u, ry - 3 * u, cx + u, ry - u, 0x6b4a30);
}
