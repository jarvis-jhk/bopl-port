//! Extra spawn points for more than 4 teams (match-flow.md §17.2 proposal).
//!
//! Deterministic, pure Fix arithmetic, computed when a level is converted so
//! every peer agrees. Candidates in order: the midpoints of the original
//! points (0,1), (2,3), (0,2), (1,3), (0,3), (1,2), then the originals
//! mirrored in x. A candidate is kept if it lies above a platform (within
//! the platform's x extent and at most 40 units above its top, axis-aligned
//! bounds); otherwise the nearest original spawn point is used instead.

use bopl_sim::data::LevelData;
use bopl_sim::fix::Fix;
use bopl_sim::vec2::Vec2;

fn above_platform(ld: &LevelData, p: Vec2) -> bool {
    ld.platforms.iter().any(|pl| {
        let t = &pl.transform;
        let e = pl.collider.start_extents;
        let r = pl.collider.start_radius;
        let hx = t.right.x.abs() * e.x + t.up.x.abs() * e.y + r;
        let hy = t.right.y.abs() * e.x + t.up.y.abs() * e.y + r;
        let top = t.position.y + hy;
        (p.x - t.position.x).abs() <= hx && p.y > top && p.y - top < Fix::from_int(40)
    })
}

/// `n` generated spawn points (team slots 4, 5, ...).
pub fn generate(ld: &LevelData, n: usize) -> Vec<Vec2> {
    let s = &ld.team_spawns;
    let half = Fix(2_147_483_648);
    let mid = |a: usize, b: usize| Vec2::new((s[a].x + s[b].x) * half, (s[a].y + s[b].y) * half);
    let mut cands = vec![
        mid(0, 1),
        mid(2, 3),
        mid(0, 2),
        mid(1, 3),
        mid(0, 3),
        mid(1, 2),
    ];
    cands.extend(s.iter().map(|p| Vec2::new(-p.x, p.y)));
    cands
        .into_iter()
        .take(n)
        .map(|c| {
            if above_platform(ld, c) {
                c
            } else {
                *s.iter()
                    .min_by_key(|p| (**p - c).sqr_magnitude())
                    .unwrap_or(&s[0])
            }
        })
        .collect()
}
