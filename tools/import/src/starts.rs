//! Start spots for a streamed district (`game_core::city::Index::starts`): places a rider can be put down
//! and roll away from. Skate School's whole-district spawn rule (the flat face nearest the origin on the
//! main floor) lands under the island or in the air in a city, so the district's collision is searched
//! instead.
//!
//! A spot is a walkable surface on a 4 m grid (every level of it: Skate School has a walkable lid over
//! the whole island), flat and rideable (not a surface the board can't ride, not stairs), with headroom,
//! room all round, a long straight run in some direction (no wall, no step up or drop, no steep slope)
//! and rails near it at about its own height, so it is street level among things to skate (not a roof,
//! the seabed under an island, or an empty lot). It faces along its longest run. The first start is the
//! good spot with the most rails around; each next one the good spot farthest from those already
//! picked, at least [`APART`] from them.

use crate::park::RailPoints;
use crate::world::Triangle;
use game_core::collision::{self, TriMesh};
use game_core::math::{Vec3, v3};
use game_core::park::{STAIRS, Terrain, surface_category};
use std::collections::HashMap;

/// Spacing of the spots tried (m).
const GRID: f32 = 4.0;
/// Levels tried at a grid point, top down, and the headroom a spot needs (m).
const LEVELS: usize = 8;
const HEADROOM: f32 = 2.5;
/// Good spots: scoring at least this share of the best ...
const GOOD: f32 = 0.9;
/// ... or, when those run out, this share.
const FAIR: f32 = 0.6;
/// A straight run counts up to this long (m).
const RUN: f32 = 40.0;
/// The shortest best run a spot may have (m).
const MIN_RUN: f32 = 20.0;
/// Clear ground all round a spot (m).
const ROOM: f32 = 3.0;
/// Starts are at least this far apart (m).
pub const APART: f32 = 200.0;
/// Rails counted within this distance of a spot (m) ...
const RAIL_REACH: f32 = 60.0;
/// ... with their middle within this height of the spot's (m).
const RAIL_LEVEL: f32 = 4.0;
/// Fewest such rails for a spot.
const MIN_RAILS: usize = 3;
/// Most rails that count towards a spot's score.
const RAILS_COUNTED: usize = 15;
/// Walking a run: step (m), highest step up and down (m), steepest slope (normal's y).
const STEP: f32 = 1.0;
const STEP_UP: f32 = 0.3;
const STEP_DOWN: f32 = 0.5;
const FLAT_Y: f32 = 0.97;
const SLOPE_Y: f32 = 0.94;
/// Surface category the board can't ride (`game_core::park::surface_category`).
const UNRIDEABLE: u32 = 6;

/// Directions tried: 16, yaw 0 facing +z, positive towards +x.
fn dir(k: usize) -> (Vec3, f32) {
    let yaw = k as f32 * std::f32::consts::TAU / 16.0;
    (v3(yaw.sin(), 0.0, yaw.cos()), yaw)
}

struct Spot {
    at: Vec3,
    yaw: f32,
    score: f32,
    rails: usize,
}

/// Up to `max` starts, best first. None when the district has nowhere that passes.
pub fn pick(tris: &[Triangle], rails: &[RailPoints], max: usize) -> Vec<(Vec3, f32)> {
    let t: Vec<([[f32; 3]; 3], u16, u16)> = tris
        .iter()
        .map(|t| (t.v, t.surface, t.flags as u16))
        .collect();
    let Ok(mesh) = TriMesh::parse(&collision::encode(&t), (Vec3::ZERO, 0.0)) else {
        return Vec::new();
    };
    // Rail middles in 20 m buckets.
    let bucket = |x: f32, z: f32| ((x / 20.0).floor() as i32, (z / 20.0).floor() as i32);
    let mut near: HashMap<(i32, i32), Vec<Vec3>> = HashMap::new();
    for r in rails {
        if let Some(p) = r.get(r.len() / 2) {
            near.entry(bucket(p[0], p[2]))
                .or_default()
                .push(v3(p[0], p[1], p[2]));
        }
    }
    let rails_by = |p: Vec3| {
        let (bx, bz) = bucket(p.x, p.z);
        let reach = (RAIL_REACH / 20.0).ceil() as i32;
        let mut n = 0;
        for i in -reach..=reach {
            for j in -reach..=reach {
                for r in near.get(&(bx + i, bz + j)).into_iter().flatten() {
                    let (dx, dz) = (r.x - p.x, r.z - p.z);
                    if dx * dx + dz * dz <= RAIL_REACH * RAIL_REACH
                        && (r.y - p.y).abs() <= RAIL_LEVEL
                    {
                        n += 1;
                    }
                }
            }
        }
        n
    };
    let top = mesh.max.y + 1.0;
    let mut spots = Vec::new();
    let mut x = (mesh.min.x / GRID).ceil() * GRID;
    while x <= mesh.max.x {
        let mut z = (mesh.min.z / GRID).ceil() * GRID;
        while z <= mesh.max.z {
            let mut below = top;
            for _ in 0..LEVELS {
                let g = mesh.ground(x, z, below);
                if g.height == f32::MIN {
                    break;
                }
                if g.normal.y >= FLAT_Y
                    && let Some(s) = spot(&mesh, v3(x, g.height, z), &rails_by)
                {
                    spots.push(s);
                }
                below = g.height - HEADROOM;
            }
            z += GRID;
        }
        x += GRID;
    }
    let best = spots.iter().map(|s| s.score).fold(0.0, f32::max);
    let flat = |a: Vec3, b: Vec3| ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt();
    let mut out: Vec<(Vec3, f32)> = Vec::new();
    if let Some(first) = spots
        .iter()
        .filter(|s| s.score >= GOOD * best)
        .max_by(|a, b| (a.rails, a.score).partial_cmp(&(b.rails, b.score)).unwrap())
    {
        out.push((first.at, first.yaw));
    }
    while out.len() < max {
        let gap = |s: &Spot| {
            out.iter()
                .map(|(p, _)| flat(*p, s.at))
                .fold(f32::MAX, f32::min)
        };
        let next = [GOOD, FAIR].iter().find_map(|&share| {
            spots
                .iter()
                .filter(|s| s.score >= share * best && gap(s) >= APART)
                .max_by(|a, b| gap(a).total_cmp(&gap(b)))
        });
        match next {
            Some(s) => out.push((s.at, s.yaw)),
            None => break,
        }
    }
    out
}

/// The spot at `at` (on a flat walkable face), if it passes.
fn spot(mesh: &TriMesh, at: Vec3, rails_by: &impl Fn(Vec3) -> usize) -> Option<Spot> {
    // Headroom, and a rideable surface.
    if mesh.ray(at + Vec3::UP * 0.05, Vec3::UP, HEADROOM).is_some() {
        return None;
    }
    let down = mesh.ray(at + Vec3::UP * 0.3, v3(0.0, -1.0, 0.0), 0.4)?;
    let cat = surface_category(down.surface);
    if cat == UNRIDEABLE || cat == STAIRS {
        return None;
    }
    let rails = rails_by(at);
    if rails < MIN_RAILS {
        return None;
    }
    // Room all round first (cheap), then the runs.
    if (0..16)
        .step_by(2)
        .any(|k| run(mesh, at, dir(k).0, ROOM) < ROOM)
    {
        return None;
    }
    let runs: Vec<f32> = (0..16).map(|k| run(mesh, at, dir(k).0, RUN)).collect();
    let (best, &len) = runs.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if len < MIN_RUN {
        return None;
    }
    let back = runs[(best + 8) % 16];
    let score = len + 0.5 * back + 2.0 * rails.min(RAILS_COUNTED) as f32;
    Some(Spot {
        at,
        yaw: dir(best).1,
        score,
        rails,
    })
}

/// How far a rider can roll from `at` along `d` (up to `max`): stopping at a wall, a step up or drop, a
/// steep slope or the end of the ground.
fn run(mesh: &TriMesh, at: Vec3, d: Vec3, max: f32) -> f32 {
    let mut p = at;
    let mut gone = 0.0;
    while gone < max {
        let q = p + d * STEP;
        let g = mesh.ground(q.x, q.z, p.y + STEP_UP);
        if g.height == f32::MIN || p.y - g.height > STEP_DOWN || g.normal.y < SLOPE_Y {
            break;
        }
        let q = v3(q.x, g.height, q.z);
        if mesh.blocked(p, q) {
            break;
        }
        p = q;
        gone += STEP;
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(x0: f32, z0: f32, x1: f32, z1: f32, y: f32, surface: u16) -> [Triangle; 2] {
        let t = |v| Triangle {
            v,
            surface,
            flags: 0x80,
        };
        [
            t([[x0, y, z0], [x0, y, z1], [x1, y, z1]]),
            t([[x0, y, z0], [x1, y, z1], [x1, y, z0]]),
        ]
    }

    /// A wall from (x, z0) to (x, z1), `h` high, standing on y.
    fn wall(x: f32, z0: f32, z1: f32, y: f32, h: f32) -> [Triangle; 2] {
        let t = |v| Triangle {
            v,
            surface: 0,
            flags: 0,
        };
        [
            t([[x, y, z0], [x, y + h, z0], [x, y + h, z1]]),
            t([[x, y, z0], [x, y + h, z1], [x, y, z1]]),
        ]
    }

    fn rails_at(x: f32, y: f32, z: f32, n: usize) -> Vec<RailPoints> {
        (0..n)
            .map(|i| {
                let z = z + i as f32 * 3.0;
                vec![
                    [x - 2.0, y + 0.5, z],
                    [x, y + 0.5, z],
                    [x + 2.0, y + 0.5, z],
                ]
            })
            .collect()
    }

    #[test]
    fn a_street_among_rails_not_the_seabed_a_roof_or_a_lid() {
        let mut tris = Vec::new();
        // The seabed: wide, flat and low, nothing on it.
        tris.extend(quad(-400.0, -400.0, 400.0, 400.0, -70.0, 0));
        // The island's street: 120 m along x, 16 m wide, rails beside it.
        tris.extend(quad(0.0, 0.0, 120.0, 16.0, 0.0, 0));
        // A roof beside the street, 10 m up, as long, rails on the street below only.
        tris.extend(quad(0.0, 30.0, 120.0, 60.0, 10.0, 0));
        // A walkable lid over everything (as Skate School has).
        tris.extend(quad(-500.0, -500.0, 500.0, 500.0, 80.0, 0));
        let rails = rails_at(60.0, 0.0, 2.0, 5);
        let starts = pick(&tris, &rails, 4);
        assert!(!starts.is_empty());
        let (p, yaw) = starts[0];
        assert!(p.y.abs() < 1e-3, "on the street: {p:?}");
        assert!(
            (0.0..=16.0).contains(&p.z) && (0.0..=120.0).contains(&p.x),
            "{p:?}"
        );
        // Facing along the street (+x or -x).
        assert!(yaw.sin().abs() > 0.99, "along the street: yaw {yaw}");
        assert!(
            starts.iter().all(|(p, _)| p.y.abs() < 1e-3),
            "only the street: {starts:?}"
        );
    }

    #[test]
    fn walls_grass_and_spacing() {
        let mut tris = Vec::new();
        // Two streets 300 m apart, each with rails; the first walled across every 15 m (no 20 m run).
        tris.extend(quad(0.0, 0.0, 120.0, 16.0, 0.0, 0));
        for k in 1..8 {
            tris.extend(wall(k as f32 * 15.0, -1.0, 17.0, 0.0, 2.0));
        }
        tris.extend(quad(0.0, 300.0, 120.0, 320.0, 0.0, 0));
        // A third street of a surface the board can't ride.
        tris.extend(quad(
            0.0,
            600.0,
            120.0,
            620.0,
            0.0,
            (UNRIDEABLE << 7) as u16,
        ));
        // A fourth, rideable, 100 m from the second: too close for a second start.
        tris.extend(quad(0.0, 400.0, 120.0, 420.0, 0.0, 0));
        let mut rails = rails_at(60.0, 0.0, 5.0, 5);
        rails.extend(rails_at(60.0, 0.0, 305.0, 5));
        rails.extend(rails_at(60.0, 0.0, 605.0, 5));
        rails.extend(rails_at(60.0, 0.0, 405.0, 5));
        let starts = pick(&tris, &rails, 4);
        assert_eq!(starts.len(), 1, "{starts:?}");
        let p = starts[0].0;
        assert!((300.0..=420.0).contains(&p.z), "{p:?}");
    }

    #[test]
    fn nowhere_without_rails() {
        let tris = quad(0.0, 0.0, 120.0, 20.0, 0.0, 0);
        assert!(pick(&tris, &[], 4).is_empty());
    }
}
