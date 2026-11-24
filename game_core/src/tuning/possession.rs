//! Tuning for the board off the rider's feet: Skate 3's `physics_skatecontroller` (dropping, throwing,
//! the board left behind and picking it back up), the released board's contact material
//! (`physics_wipeout/SkateboardFriction`, `SkateboardRestitution`) and the landing onto it from the air
//! (`physics_landingondeck`, `physics_wipeout/MaxSpeedLandingOnBoard`), as SK8-ENGINE/skate-3-rust-engine
//! reads them (GPL-3.0; skate-game physics/offboard/board_possession/settings.rs,
//! physics/offboard/landing_deck.rs, physics/landing_on_deck.rs). The built-in values are Skate 3's stock
//! ones (read off the disc's `skatercollections` with `fg_import values`); the rules pack carries the same
//! paths.

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PossessionTuning {
    /// A free board further than this from the rider (m) is hidden (`MaxDistance`), parked this far
    /// away along the way to it (`DistanceToHideSkateboard`: out of sight).
    pub hide_distance: f32,
    pub hide_offset: f32,
    /// A retrieval starts no further than this from the hand (m): a board further away is moved in to
    /// it first (`DistanceForBoardReturn`), or this while getting on (`DistanceForMountedReturn`).
    pub return_distance: f32,
    pub mounted_return_distance: f32,
    /// Getting on, the retrieval takes at most this long (s, `MinTimeToRetrieveBoardWhileMounting`).
    pub mounting_time: f32,
    /// The retrieval's time (s) against the distance (m) and its weight (0..1, from the start to the
    /// hand) against its progress (0..1): `RetrievalTimeVsDist`, `RetrievalSpeedCurve`.
    pub retrieval_time: Curve,
    pub retrieval_weight: Curve,
    /// A throw: launched this many degrees up from the facing (`ThrownSkateboardLaunchPitch`) at the
    /// rider's speed plus `ThrownSkateboardVelocity` of it (m/s against m/s).
    pub throw_pitch: f32,
    pub throw_velocity: Curve,
    /// The thrown board's attitude: turned towards this pitch (degrees, nose down) and its heading,
    /// rolled level, each by its scalar of the angle left a tick, for the throw's first 8 ticks
    /// (`ThrownSkateboardPitchTarget`, `...PitchScalar`, `...RollScalar`, `...YawScalar`).
    pub throw_target_pitch: f32,
    pub throw_pitch_scalar: f32,
    pub throw_roll_scalar: f32,
    pub throw_yaw_scalar: f32,
    /// The released board's contact material: friction and restitution.
    pub friction: f32,
    pub restitution: f32,
    /// Landing onto the board from the air: its up must be this upright (`DeckMinUprightness`), the
    /// rider's centre of mass is aimed this high over it (`ApproxCOMHeightOnLanding`, m), and landing
    /// faster than this (m/s) bails (`MaxSpeedLandingOnBoard`).
    pub deck_min_up: f32,
    pub landing_com_height: f32,
    pub max_landing_speed: f32,
}

impl PossessionTuning {
    /// Skate 3's stock values.
    pub const BUILT_IN: PossessionTuning = PossessionTuning {
        hide_distance: 30.0,
        hide_offset: 1000.0,
        return_distance: 30.0,
        mounted_return_distance: 5.0,
        mounting_time: 0.1,
        retrieval_time: Curve::new(&[
            (0.0, 0.128_571_4),
            (3.013_029, 0.357_143),
            (8.957_655, 0.603_571_5),
            (18.159_61, 0.742_857),
            (26.628_66, 0.817_857),
            (34.527_69, 0.867_857),
            (41.205_21, 0.925),
            (50.0, 1.0),
        ]),
        retrieval_weight: Curve::new(&[
            (0.0, 0.0),
            (0.201_954_4, 0.039_285_72),
            (0.335_504_9, 0.078_571_45),
            (0.547_231_3, 0.192_857_1),
            (0.745_928_3, 0.417_857_1),
            (0.838_762_2, 0.582_143),
            (0.967_426_7, 0.975),
            (1.0, 1.0),
        ]),
        throw_pitch: 18.0,
        throw_velocity: Curve::new(&[
            (0.0, 3.0),
            (2.019_544, 2.067_857),
            (2.980_456, 1.703_571),
            (3.876_221, 1.403_571),
            (4.869_707, 1.167_857),
            (5.993_485, 0.921_428_6),
            (7.736_156, 0.664_285_7),
            (9.022_801, 0.535_714_3),
        ]),
        throw_target_pitch: 20.0,
        throw_pitch_scalar: 0.1,
        throw_roll_scalar: 0.1,
        throw_yaw_scalar: 0.2,
        friction: 0.36,
        restitution: 0.0,
        deck_min_up: 0.71,
        landing_com_height: 0.82,
        max_landing_speed: 11.5,
    };

    pub(super) fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>) -> bool {
        let scalar = |field: &mut f32| match f {
            Some(v) => {
                *field = v;
                true
            }
            None => false,
        };
        let curve = |field: &mut Curve| match c {
            Some(v) => {
                *field = v;
                true
            }
            None => false,
        };
        match name {
            "physics_skatecontroller/default/MaxDistance" => scalar(&mut self.hide_distance),
            "physics_skatecontroller/default/DistanceToHideSkateboard" => {
                scalar(&mut self.hide_offset)
            }
            "physics_skatecontroller/default/DistanceForBoardReturn" => {
                scalar(&mut self.return_distance)
            }
            "physics_skatecontroller/default/DistanceForMountedReturn" => {
                scalar(&mut self.mounted_return_distance)
            }
            "physics_skatecontroller/default/MinTimeToRetrieveBoardWhileMounting" => {
                scalar(&mut self.mounting_time)
            }
            "physics_skatecontroller/default/RetrievalTimeVsDist" => {
                curve(&mut self.retrieval_time)
            }
            "physics_skatecontroller/default/RetrievalSpeedCurve" => {
                curve(&mut self.retrieval_weight)
            }
            "physics_skatecontroller/default/ThrownSkateboardLaunchPitch" => {
                scalar(&mut self.throw_pitch)
            }
            "physics_skatecontroller/default/ThrownSkateboardVelocity" => {
                curve(&mut self.throw_velocity)
            }
            "physics_skatecontroller/default/ThrownSkateboardPitchTarget" => {
                scalar(&mut self.throw_target_pitch)
            }
            "physics_skatecontroller/default/ThrownSkateboardPitchScalar" => {
                scalar(&mut self.throw_pitch_scalar)
            }
            "physics_skatecontroller/default/ThrownSkateboardRollScalar" => {
                scalar(&mut self.throw_roll_scalar)
            }
            "physics_skatecontroller/default/ThrownSkateboardYawScalar" => {
                scalar(&mut self.throw_yaw_scalar)
            }
            "physics_wipeout/default/SkateboardFriction" => scalar(&mut self.friction),
            "physics_wipeout/default/SkateboardRestitution" => scalar(&mut self.restitution),
            "physics_landingondeck/default/DeckMinUprightness" => scalar(&mut self.deck_min_up),
            "physics_landingondeck/default/ApproxCOMHeightOnLanding" => {
                scalar(&mut self.landing_com_height)
            }
            "physics_wipeout/default/MaxSpeedLandingOnBoard" => scalar(&mut self.max_landing_speed),
            _ => false,
        }
    }
}

pub const PATHS: &[&str] = &[
    "physics_skatecontroller/default/MaxDistance",
    "physics_skatecontroller/default/DistanceToHideSkateboard",
    "physics_skatecontroller/default/DistanceForBoardReturn",
    "physics_skatecontroller/default/DistanceForMountedReturn",
    "physics_skatecontroller/default/MinTimeToRetrieveBoardWhileMounting",
    "physics_skatecontroller/default/RetrievalTimeVsDist",
    "physics_skatecontroller/default/RetrievalSpeedCurve",
    "physics_skatecontroller/default/ThrownSkateboardLaunchPitch",
    "physics_skatecontroller/default/ThrownSkateboardVelocity",
    "physics_skatecontroller/default/ThrownSkateboardPitchTarget",
    "physics_skatecontroller/default/ThrownSkateboardPitchScalar",
    "physics_skatecontroller/default/ThrownSkateboardRollScalar",
    "physics_skatecontroller/default/ThrownSkateboardYawScalar",
    "physics_wipeout/default/SkateboardFriction",
    "physics_wipeout/default/SkateboardRestitution",
    "physics_landingondeck/default/DeckMinUprightness",
    "physics_landingondeck/default/ApproxCOMHeightOnLanding",
    "physics_wipeout/default/MaxSpeedLandingOnBoard",
];
