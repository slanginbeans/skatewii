//! Placeholder skater and board built from boxes. Board space: +Z is the nose, +Y up, wheels touch
//! y = 0. The rider stands side-on, facing +X.

use alloc::vec::Vec;

use crate::gfx::{MeshBuilder, Vertex, rgb};
use crate::math::v3;

pub struct Models {
    pub board: Vec<Vertex>,
    pub rider: Vec<Vertex>,
    pub rider_crouched: Vec<Vertex>,
}

const DECK: u32 = rgb(40, 140, 120);
const GRIP: u32 = rgb(34, 34, 38);
const WHEEL: u32 = rgb(236, 226, 190);
const TRUCK: u32 = rgb(150, 150, 158);
const PANTS: u32 = rgb(52, 62, 96);
const SHIRT: u32 = rgb(232, 150, 40);
const SKIN: u32 = rgb(206, 156, 120);
const SHOES: u32 = rgb(230, 230, 230);
const CAP: u32 = rgb(180, 40, 60);

fn board() -> Vec<Vertex> {
    let mut m = MeshBuilder::new();
    m.cuboid(v3(-0.105, 0.085, -0.40), v3(0.105, 0.10, 0.40), DECK);
    m.cuboid(v3(-0.10, 0.10, -0.39), v3(0.10, 0.104, 0.39), GRIP);
    for z in [-0.26, 0.26] {
        m.cuboid(
            v3(-0.08, 0.055, z - 0.025),
            v3(0.08, 0.085, z + 0.025),
            TRUCK,
        );
        for x in [-0.09, 0.09] {
            m.cuboid(
                v3(x - 0.02, 0.0, z - 0.028),
                v3(x + 0.02, 0.056, z + 0.028),
                WHEEL,
            );
        }
    }
    m.verts
}

/// `crouch` 0 = standing, 1 = loaded for a pop.
fn rider(crouch: f32) -> Vec<Vertex> {
    let mut m = MeshBuilder::new();
    let foot = 0.104;
    let hip = 0.88 - 0.30 * crouch;
    let shoulder = hip + 0.55 - 0.08 * crouch;
    // Feet on the bolts, legs, hips.
    for z in [-0.22, 0.20] {
        m.cuboid(
            v3(-0.06, foot, z - 0.12),
            v3(0.07, foot + 0.07, z + 0.03),
            SHOES,
        );
        m.cuboid(
            v3(-0.055, foot + 0.07, z - 0.06),
            v3(0.055, hip, z + 0.04),
            PANTS,
        );
    }
    m.cuboid(
        v3(-0.10, hip - 0.06, -0.26),
        v3(0.10, hip + 0.04, 0.24),
        PANTS,
    );
    // Torso leans forward when crouched.
    let lean = 0.10 * crouch;
    m.cuboid(
        v3(-0.11 + lean, hip, -0.18),
        v3(0.11 + lean, shoulder, 0.17),
        SHIRT,
    );
    // Arms out for balance.
    m.cuboid(
        v3(-0.05 + lean, shoulder - 0.10, -0.62),
        v3(0.05 + lean, shoulder - 0.02, -0.18),
        SHIRT,
    );
    m.cuboid(
        v3(-0.05 + lean, shoulder - 0.10, 0.17),
        v3(0.05 + lean, shoulder - 0.02, 0.60),
        SHIRT,
    );
    m.cuboid(
        v3(-0.045 + lean, shoulder - 0.095, -0.70),
        v3(0.045 + lean, shoulder - 0.025, -0.62),
        SKIN,
    );
    m.cuboid(
        v3(-0.045 + lean, shoulder - 0.095, 0.60),
        v3(0.045 + lean, shoulder - 0.025, 0.68),
        SKIN,
    );
    // Head and cap.
    m.cuboid(
        v3(-0.09 + lean, shoulder + 0.03, -0.09),
        v3(0.09 + lean, shoulder + 0.23, 0.09),
        SKIN,
    );
    m.cuboid(
        v3(-0.10 + lean, shoulder + 0.19, -0.10),
        v3(0.10 + lean, shoulder + 0.26, 0.10),
        CAP,
    );
    m.cuboid(
        v3(0.10 + lean, shoulder + 0.19, -0.08),
        v3(0.20 + lean, shoulder + 0.21, 0.08),
        CAP,
    );
    m.verts
}

impl Models {
    pub fn build() -> Models {
        Models {
            board: board(),
            rider: rider(0.0),
            rider_crouched: rider(1.0),
        }
    }
}
