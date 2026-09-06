//! Print the seeded map as glyphs, with its places marked, to look at a
//! layout before anyone walks it: `cargo run -p cqs --example map -- 7`.
use world::World;

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let w = World::new(seed);
    let mut rows: Vec<Vec<char>> = w.ascii().lines().map(|l| l.chars().collect()).collect();
    for (i, p) in w.places.iter().enumerate() {
        if let Some(row) = rows.get_mut(p.y as usize) {
            if let Some(c) = row.get_mut(p.x as usize) {
                *c = char::from_digit(i as u32, 10).unwrap_or('*');
            }
        }
        println!("{i} {} ({},{})", p.name, p.x, p.y);
    }
    for row in rows {
        println!("{}", row.into_iter().collect::<String>());
    }
}
