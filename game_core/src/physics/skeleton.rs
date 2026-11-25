//! The rider's physical skeleton (SK8-ENGINE/skate-3-rust-engine, GPL-3.0: skate-core
//! physics/skeleton_body/construction.rs, `SkeletonBody::Init` 0x82BE4900 and its extra capsules
//! 0x82BE5180; construction/primitive_mass.rs; skate-game physics/skeleton_body.rs `load`).
//!
//! Skate 3 simulates the rider as 26 rigid bodies: one for each of the 24 bones of its physical
//! skeleton (`PHYS_TPOSE`, the skateboard root first), driven towards the animated pose, and two
//! extra capsules. Each bone's volume is a sphere or a capsule sized from the bone's box, its mass the
//! box's volume times `MassOfSkeleton` and the bone's mass factor (the ragdoll has its own factor),
//! its inertia the shape's, divided by `InertiaMultFactor`. The values here are the stock ones
//! (`physics_skeleton`, `physics_skeleton_bones`, `animation`, the `PHYS_TPOSE` boxes); the head's
//! hat cylinder (`physics_hat`, a customised rider's) is not modelled.

use super::body::Mass;
use crate::math::{Vec3, v3};

/// Bodies: the animated bones, then the two extra capsules.
pub const ANIMATED_PARTS: usize = 24;
pub const PARTS: usize = 26;

/// A bone's volume (`physics_skeleton_bones` `VolumeType`): its box, a sphere about its larger
/// cross-section, a capsule along it, or the skateboard root's own capsule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Volume {
    Box,
    Sphere,
    Capsule,
    Root,
}

/// A bone's settings (`physics_skeleton_bones/default/PART_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneSettings {
    pub mass_factor: f32,
    pub ragdoll_mass_factor: f32,
    pub has_collision: bool,
    pub use_root_drive: bool,
    pub volume: Volume,
    pub volume_scalar: f32,
    pub parents: u32,
}

/// `physics_skeleton/default` (`MassOfSkeleton`, `SkateRootCapsuleRadius`, `SkateRootCapsuleLength`,
/// `CapsuleLengthScalar`), `physics_skeleton_bones/default/CapsuleRadiusScalar`,
/// `physics_wipeout/default/RagdollInvMassFactor` and `animation/default` (`InertiaMultiplyType`,
/// `InertiaMultFactor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletonSettings {
    pub density: f32,
    pub root_radius: f32,
    pub root_half_length: f32,
    pub capsule_radius_scalar: f32,
    pub capsule_length_scalar: f32,
    pub ragdoll_inverse_mass_factor: f32,
    pub inertia_multiply_type: u32,
    pub inertia_factor: f32,
}

impl SkeletonSettings {
    pub const STOCK: SkeletonSettings = SkeletonSettings {
        density: 320.0,
        root_radius: 0.45,
        root_half_length: 0.1,
        capsule_radius_scalar: 1.2,
        capsule_length_scalar: 1.2,
        ragdoll_inverse_mass_factor: 1.0,
        inertia_multiply_type: 1,
        inertia_factor: 5.0,
    };
}

/// A body's shape (SK8 `MassShape`): a capsule's axis is its local z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere { radius: f32 },
    Capsule { radius: f32, half_length: f32 },
    Box { half_extents: Vec3 },
}

/// One body: its shape and its mass animated (riding) and as a ragdoll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub shape: Shape,
    pub animated: Mass,
    pub ragdoll: Mass,
}

/// The physical skeleton's bones, in Skate 3's order (`PHYS_TPOSE`).
pub const BONE_NAMES: [&str; ANIMATED_PARTS] = [
    "SKATEBOARD_ROOT",
    "NECK1",
    "NECK",
    "LEFTHAND",
    "LEFTFOREARM",
    "LEFTARM",
    "LEFTSHOULDER",
    "RIGHTHAND",
    "RIGHTFOREARM",
    "RIGHTARM",
    "RIGHTSHOULDER",
    "SPINE3",
    "SPINE2",
    "SPINE1",
    "SPINE",
    "LEFTTOEBASE",
    "LEFTFOOT",
    "LEFTLEG",
    "LEFTUPLEG",
    "RIGHTTOEBASE",
    "RIGHTFOOT",
    "RIGHTLEG",
    "RIGHTUPLEG",
    "HIPS",
];

/// Each bone's box (m, x y z): `PHYS_TPOSE`'s bone sizes.
pub const BONE_SIZES: [Vec3; ANIMATED_PARTS] = [
    v3(0.2432, 0.02, 0.877),
    v3(0.18239997, 0.1828, 0.1574),
    v3(0.057600003, 0.1116, 0.122),
    v3(0.14699997, 0.019999998, 0.11439998),
    v3(0.08399997, 0.083999984, 0.23613918),
    v3(0.103999965, 0.10399999, 0.28978062),
    v3(0.17040001, 0.09299998, 0.09359999),
    v3(0.147, 0.02, 0.1144),
    v3(0.083999984, 0.08400001, 0.23619998),
    v3(0.10399998, 0.10399998, 0.28199995),
    v3(0.17040001, 0.092999995, 0.0936),
    v3(0.1286, 0.1692, 0.35619995),
    v3(0.11859999, 0.16779996, 0.35179996),
    v3(0.09639999, 0.19339998, 0.3128),
    v3(0.09419999, 0.21100001, 0.29859996),
    v3(0.064, 0.047999997, 0.099999994),
    v3(0.15893978, 0.07659999, 0.08339999),
    v3(0.11399998, 0.113999985, 0.3931457),
    v3(0.13199995, 0.13199998, 0.38499993),
    v3(0.06399999, 0.047999997, 0.099999994),
    v3(0.159, 0.0766, 0.08339999),
    v3(0.11399998, 0.11399998, 0.39319992),
    v3(0.13199995, 0.13199998, 0.38499993),
    v3(0.17219998, 0.18059997, 0.31359997),
];

/// `physics_skeleton_bones/default/PART_*`, in [`BONE_NAMES`] order.
pub const BONES: [BoneSettings; ANIMATED_PARTS] = [
    BoneSettings {
        mass_factor: 1.0,
        ragdoll_mass_factor: 1.0,
        has_collision: false,
        use_root_drive: true,
        volume: Volume::Root,
        volume_scalar: 1.0,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.5,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.2,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 4.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 0.8,
        parents: 2,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 2.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.2,
        parents: 1,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 2.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.2,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 2.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 0.9,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 4.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.0,
        parents: 2,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 2.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.2,
        parents: 1,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 2.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.2,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 2.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 0.9,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.2,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.4,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.6,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.4,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 4.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.7,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 4.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 0.9,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.25,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.25,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 4.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.7,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 4.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 0.85,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.25,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 4.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Capsule,
        volume_scalar: 1.25,
        parents: 0,
    },
    BoneSettings {
        mass_factor: 32.0,
        ragdoll_mass_factor: 1.0,
        has_collision: true,
        use_root_drive: true,
        volume: Volume::Sphere,
        volume_scalar: 1.9,
        parents: 0,
    },
];

/// The smallest radius a primitive's mass takes (SK8 primitive_mass.rs `MIN_RADIUS`).
const MIN_RADIUS: f32 = 1.0e-7;

/// A shape's principal moments per unit mass (SK8 construction/primitive_mass.rs `primitive_mass`,
/// TU3 0x82AE7228; the rounded box's 0x82AE6B78 with no rounding).
fn moments(shape: Shape) -> Vec3 {
    match shape {
        Shape::Sphere { radius } => {
            let r = radius.max(MIN_RADIUS);
            let m = r * r * 0.4;
            v3(m, m, m)
        }
        Shape::Capsule {
            radius,
            half_length,
        } => {
            let h = half_length;
            let r = radius.max(MIN_RADIUS).max(h * 0.05);
            let numerator = ((r * 1.6 + h * 0.75) * r + h * h * 4.0) * r + h * h * h;
            let transverse = numerator / (r * 4.0 + h * 3.0);
            v3(transverse, transverse, r * r * 0.4)
        }
        Shape::Box { half_extents: e } => {
            let largest = e.x.max(e.y).max(e.z);
            let floor = (largest * 0.05).max(MIN_RADIUS);
            let (x, y, z) = (e.x.max(floor), e.y.max(floor), e.z.max(floor));
            v3(
                (z * z + y * y) / 3.0,
                (x * x + z * z) / 3.0,
                (x * x + y * y) / 3.0,
            )
        }
    }
}

/// A body of `mass` with `shape`'s inertia (SK8 mass.rs `primitive_mass_properties`), its inverse
/// inertia divided by `factor` (SK8 construction.rs `adjust_inertia`, `DebugMultiplyInertiaTensor`;
/// type 2, Skate 3's spherical average, is not used by the stock settings).
fn body_mass(shape: Shape, mass: f32, factor: Option<f32>) -> Mass {
    let m = moments(shape);
    let k = factor.map_or(1.0, |f| 1.0 / f);
    Mass {
        inverse_inertia: v3(k / (m.x * mass), k / (m.y * mass), k / (m.z * mass)),
        inverse_mass: 1.0 / mass,
        max_speed: f32::MAX,
        max_spin: f32::MAX,
        linear_drag: 0.0,
        angular_drag: 0.0,
    }
}

/// Builds the 26 bodies (SK8 construction.rs `SkeletonBodyDefinition::new`).
pub fn parts(
    sizes: &[Vec3; ANIMATED_PARTS],
    bones: &[BoneSettings; ANIMATED_PARTS],
    s: &SkeletonSettings,
) -> [Part; PARTS] {
    let factor = |kind: u32, f: f32| (kind == 1).then_some(f);
    core::array::from_fn(|i| {
        if i >= ANIMATED_PARTS {
            // The two extra capsules (0x82BE4F80): fixed sizes and masses, speeds capped at 30 m/s
            // and rad/s, inertia divided by 5.
            let (radius, half_length, mass) = if i == ANIMATED_PARTS {
                (0.23, 0.1, 0.1)
            } else {
                (0.36, 0.26, 0.01)
            };
            let shape = Shape::Capsule {
                radius,
                half_length,
            };
            let mut m = body_mass(shape, mass, Some(5.0));
            m.max_speed = 29.999_998;
            m.max_spin = 29.999_998;
            return Part {
                shape,
                animated: m,
                ragdoll: m,
            };
        }
        let b = bones[i];
        let size = sizes[i];
        let shape = match b.volume {
            Volume::Root => Shape::Capsule {
                radius: s.root_radius,
                half_length: s.root_half_length,
            },
            Volume::Sphere => Shape::Sphere {
                radius: (size.x * 0.5).max(size.y * 0.5) * b.volume_scalar,
            },
            Volume::Box => Shape::Box {
                half_extents: size * (b.volume_scalar * 0.5),
            },
            Volume::Capsule => {
                let radius = size.x * s.capsule_radius_scalar * 0.5;
                let length = (size.z * 0.5 - radius).max(0.0);
                Shape::Capsule {
                    radius: b.volume_scalar * radius,
                    half_length: length * s.capsule_length_scalar * b.volume_scalar,
                }
            }
        };
        let base = if b.volume == Volume::Root {
            0.001
        } else {
            s.density * size.x * size.y * size.z
        };
        let animated = body_mass(
            shape,
            b.mass_factor * base,
            factor(s.inertia_multiply_type, s.inertia_factor),
        );
        let mut ragdoll = body_mass(shape, b.ragdoll_mass_factor * base, None);
        ragdoll.inverse_mass *= s.ragdoll_inverse_mass_factor;
        Part {
            shape,
            animated,
            ragdoll,
        }
    })
}

/// The stock skeleton's bodies.
pub fn stock_parts() -> [Part; PARTS] {
    parts(&BONE_SIZES, &BONES, &SkeletonSettings::STOCK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, what: &str) {
        assert!(
            (a - b).abs() <= b.abs() * 2e-4 + 1e-6,
            "{what}: {a} vs SK8 {b}"
        );
    }

    #[test]
    fn the_stock_skeleton_has_sk8s_bodies() {
        // SK8's SkeletonBodyDefinition for the stock data (probe dump): each body's shape, inverse
        // mass (animated, ragdoll) and inverse inertia (animated).
        let sk8: [(Shape, f32, f32, [f32; 3]); PARTS] = [
            (
                Shape::Capsule {
                    radius: 0.45,
                    half_length: 0.1,
                },
                999.99994,
                999.99994,
                [2333.4956, 2333.4956, 2469.136],
            ),
            (
                Shape::Sphere { radius: 0.1371 },
                0.14886217,
                0.5954487,
                [3.9598567; 3],
            ),
            (
                Shape::Sphere { radius: 0.06696 },
                0.99619305,
                3.9847722,
                [111.09196; 3],
            ),
            (
                Shape::Sphere { radius: 0.05879999 },
                2.322827,
                2.322827,
                [335.91733; 3],
            ),
            (
                Shape::Capsule {
                    radius: 0.060479984,
                    half_length: 0.09744423,
                },
                0.46888208,
                0.93776417,
                [13.03447, 13.03447, 64.09296],
            ),
            (
                Shape::Capsule {
                    radius: 0.07487998,
                    half_length: 0.118786074,
                },
                0.24926095,
                0.4985219,
                [4.6223655, 4.6223655, 22.227612],
            ),
            (
                Shape::Sphere {
                    radius: 0.076680005,
                },
                0.5266981,
                1.0533962,
                [44.788605; 3],
            ),
            (
                Shape::Sphere { radius: 0.0735 },
                2.322826,
                2.322826,
                [214.98692; 3],
            ),
            (
                Shape::Capsule {
                    radius: 0.060479995,
                    half_length: 0.097488,
                },
                0.46876118,
                0.93752235,
                [13.022684, 13.022684, 64.07641],
            ),
            (
                Shape::Capsule {
                    radius: 0.07487999,
                    half_length: 0.11318398,
                },
                0.25613827,
                0.51227653,
                [5.0877323, 5.0877323, 22.840887],
            ),
            (
                Shape::Sphere {
                    radius: 0.076680005,
                },
                0.526698,
                1.053396,
                [44.78859; 3],
            ),
            (
                Shape::Sphere { radius: 0.10152001 },
                0.10079868,
                0.40319473,
                [4.890143; 3],
            ),
            (
                Shape::Sphere {
                    radius: 0.117459975,
                },
                0.11158805,
                0.4463522,
                [4.04397; 3],
            ),
            (
                Shape::Sphere { radius: 0.15472 },
                0.13396454,
                0.53585815,
                [2.7981243; 3],
            ),
            (
                Shape::Sphere { radius: 0.14770001 },
                0.13163367,
                0.5265347,
                [3.0170045; 3],
            ),
            (
                Shape::Sphere {
                    radius: 0.054400004,
                },
                2.5431316,
                2.5431316,
                [429.67587; 3],
            ),
            (
                Shape::Sphere { radius: 0.0715229 },
                0.7694186,
                0.7694186,
                [75.20427; 3],
            ),
            (
                Shape::Capsule {
                    radius: 0.08549999,
                    half_length: 0.19225933,
                },
                0.15290685,
                0.6116274,
                [1.288739, 1.288739, 10.458389],
            ),
            (
                Shape::Capsule {
                    radius: 0.09899997,
                    half_length: 0.16995,
                },
                0.11646133,
                0.46584532,
                [1.1019019, 1.1019019, 5.9413013],
            ),
            (
                Shape::Sphere {
                    radius: 0.054399993,
                },
                2.5431323,
                2.5431323,
                [429.67618; 3],
            ),
            (
                Shape::Sphere { radius: 0.067575 },
                0.76912713,
                0.76912713,
                [84.21627; 3],
            ),
            (
                Shape::Capsule {
                    radius: 0.08549999,
                    half_length: 0.19229995,
                },
                0.15288578,
                0.6115431,
                [1.2881373, 1.2881373, 10.456948],
            ),
            (
                Shape::Capsule {
                    radius: 0.09899997,
                    half_length: 0.16995,
                },
                0.11646133,
                0.46584532,
                [1.1019019, 1.1019019, 5.9413013],
            ),
            (
                Shape::Sphere { radius: 0.17156997 },
                0.010013208,
                0.32042265,
                [0.17008291; 3],
            ),
            (
                Shape::Capsule {
                    radius: 0.23,
                    half_length: 0.1,
                },
                10.0,
                10.0,
                [72.54412, 72.54412, 94.51796],
            ),
            (
                Shape::Capsule {
                    radius: 0.36,
                    half_length: 0.26,
                },
                100.0,
                100.0,
                [206.66393, 206.66393, 385.80246],
            ),
        ];
        let parts = stock_parts();
        for (i, (p, (shape, im, ir, ii))) in parts.iter().zip(sk8).enumerate() {
            match (p.shape, shape) {
                (Shape::Sphere { radius: a }, Shape::Sphere { radius: b }) => close(a, b, "radius"),
                (
                    Shape::Capsule {
                        radius: a,
                        half_length: ha,
                    },
                    Shape::Capsule {
                        radius: b,
                        half_length: hb,
                    },
                ) => {
                    close(a, b, "radius");
                    close(ha, hb, "half length");
                }
                (a, b) => panic!("part {i}: {a:?} vs SK8 {b:?}"),
            }
            close(p.animated.inverse_mass, im, "inverse mass");
            close(p.ragdoll.inverse_mass, ir, "ragdoll inverse mass");
            close(p.animated.inverse_inertia.x, ii[0], "inverse inertia x");
            close(p.animated.inverse_inertia.y, ii[1], "inverse inertia y");
            close(p.animated.inverse_inertia.z, ii[2], "inverse inertia z");
        }
        // SK8's total (probe): 192.12 kg animated.
        let total: f32 = parts.iter().map(|p| 1.0 / p.animated.inverse_mass).sum();
        close(total, 192.12476, "total mass");
    }
}
