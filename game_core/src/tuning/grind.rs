//! Grind tuning: the numbers Skate 3's grinds read from its tuning database (`physics_grinds`,
//! `physics_grinds_air`, the grind fields of `physics_trajectory`, `physics_wipeout` and `physics_mode`,
//! the board's dimensions in `physicsdeck` / `physicstrucks`, and `anim_motion/grind_twist` /
//! `grind_height`). The built-in values are Skate 3's own (read off its tuning database); the player's
//! disc supplies them again through the rules pack (`fg_import rules`).

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrindTuning {
    // physics_grinds.
    /// Friction's growth over the grind (a factor against seconds grinding).
    pub friction_vs_time: Curve,
    /// Riding into a rail from below: the cross-rail speed limit's scale against the approach's slope
    /// sine.
    pub entry_vs_slope: Curve,
    /// Riding into a rail: how much of the cross-rail velocity turns along the rail, against the
    /// approach's slope sine.
    pub vert_help: Curve,
    /// Coping hold: seconds of relief against the vertical speed, times a factor against the speed along
    /// the rail.
    pub relief_vs_vy: Curve,
    pub relief_vs_speed: Curve,
    /// Lean-off: degrees against seconds past the delay, and the push off (0..1) against degrees.
    pub exit_lean: Curve,
    pub exit_assist: Curve,
    /// The lateral pins' strength (0..1) against the grind normal's up against the rail's upmost
    /// normal (cosine): a board leaning off the rail is held less (gravity relief holds it fully).
    pub pin_vs_slope: Curve,
    /// Contact boxes: a truck's half width, the deck centre to a truck, how far above and below the deck
    /// the deck box reaches (m).
    pub truck_to_wheel: f32,
    pub deck_to_truck: f32,
    pub test_above: f32,
    pub test_below: f32,

    // physics_grinds_air: the air lock-on.
    /// Angle windows (degrees): along the rail for 50-50s and 5-0s, across it for board and tip slides.
    pub angle_50: f32,
    pub angle_5o: f32,
    pub angle_board: f32,
    pub angle_tip: f32,
    /// How much of the yaw error the assist turns out, per family.
    pub yaw_50: f32,
    pub yaw_5o: f32,
    pub yaw_board: f32,
    pub yaw_tip: f32,
    /// Reach per family, in deck-centre-to-truck lengths.
    pub reach_50: f32,
    pub reach_5o: f32,
    pub reach_board: f32,
    pub reach_tip: f32,
    /// The assist's most offset (m), its most offset change a tick (m) and its most turn a tick (deg).
    pub max_offset: f32,
    pub max_offset_step: f32,
    pub max_angle_step: f32,
    /// Ticks of flight predicted, the extra downward push in the prediction along the board's up
    /// (m/s^2), and how far out on the tip the tip points are (share of the tip's length).
    pub frames: f32,
    pub stomp: f32,
    pub tip_fraction: f32,

    // The board's dimensions (m, degrees).
    pub deck_mid_length: f32,
    pub deck_end_size: f32,
    pub deck_end_angle: f32,
    pub truck_z: f32,
    pub truck_y: f32,

    // physics_trajectory and physics_mode: grind targeting in the landing prediction.
    /// How close a flight must come to a rail to be steered onto it (m), scaled on ledges by where the
    /// flight and the body are (high side, low side).
    pub lock_dist: f32,
    pub lock_ledge: [f32; 4],
    /// Most cross-rail speed squared into a ledge and into a rail, and most speed down onto it.
    pub max_speed_sq_ledge: f32,
    pub max_speed_sq_rail: f32,
    pub max_speed_down: f32,
    /// Corrections past the lock distance are shortened by this share of half the board.
    pub tip_scalar: f32,
    /// Most the flight is turned to meet the rail (degrees).
    pub adjust_max_angle: f32,
    /// How far above the rail the flight is aimed (m, along the rail's up; negative: above it).
    pub offset: f32,
    /// The landing normal on a rail: the cross-rail velocity's lean, at most this many degrees off the
    /// rail's up.
    pub landing_vel_scalar: f32,
    pub landing_max_angle: f32,
    /// A candidate flight that meets a rail scores this; near misses are penalised by distance.
    pub score: f32,
    pub penalty: Curve,

    // physics_wipeout.
    /// In a grind: the deck's closing speed (m/s) and angle error (rad) that wipe out.
    pub wipeout_xz: f32,
    pub wipeout_angle: f32,

    // anim_motion/grind_twist and grind_height: the grind trees' twist and height.
    pub twist_smoothing: f32,
    pub twist_sensitivity: f32,
    pub twist_accel: f32,
    pub twist_step: f32,
    pub height_min: f32,
    pub height_max: f32,
    pub height_speed: f32,
}

impl GrindTuning {
    /// Skate 3's values (its tuning database, as `fg_import values` reads them).
    pub const BUILT_IN: GrindTuning = GrindTuning {
        friction_vs_time: Curve::new(&[
            (0.0, 0.632_812_5),
            (0.956_701, 0.718_75),
            (2.197_938, 0.828_125),
            (4.0, 1.0),
        ]),
        entry_vs_slope: Curve::new(&[
            (0.0, 0.695_312_5),
            (0.356_701, 0.835_937_5),
            (0.744_329_9, 0.921_875),
            (1.0, 1.0),
        ]),
        vert_help: Curve::new(&[
            (0.0, 0.0),
            (0.308_247_4, 0.382_812_5),
            (0.623_711_3, 0.710_937_7),
            (0.841_237_1, 0.953_125),
        ]),
        relief_vs_vy: Curve::new(&[
            (0.0, 0.0),
            (1.164_948, 0.906_25),
            (3.134_021, 1.5),
            (10.0, 2.0),
        ]),
        relief_vs_speed: Curve::new(&[
            (0.0, 1.0),
            (3.680_413, 1.375),
            (7.195_876, 1.828_125),
            (10.0, 2.203_125),
        ]),
        exit_lean: Curve::new(&[
            (0.0, 0.0),
            (0.648_959_5, 0.0),
            (0.889_175_3, 4.5),
            (1.082_147, 14.142_86),
            (1.238_66, 28.285_72),
            (1.360_825, 29.571_43),
            (1.450_516, 29.892_86),
            (1.5, 30.214_28),
        ]),
        exit_assist: Curve::new(&[(0.0, 0.0), (11.319_59, 0.0), (56.319_59, 1.0), (90.0, 1.0)]),
        pin_vs_slope: Curve::new(&[
            (0.0, 0.0),
            (0.762_886_6, 0.0),
            (0.910_309_3, 0.368_055_6),
            (1.0, 1.0),
        ]),
        truck_to_wheel: 0.095,
        deck_to_truck: 0.243,
        test_above: 0.03,
        test_below: 0.2,
        angle_50: 16.0,
        angle_5o: 65.0,
        angle_board: 30.0,
        angle_tip: 40.0,
        yaw_50: 1.0,
        yaw_5o: 0.03,
        yaw_board: 0.6,
        yaw_tip: 1.0,
        reach_50: 0.8,
        reach_5o: 1.0,
        reach_board: 0.75,
        reach_tip: 0.7,
        max_offset: 0.2,
        max_offset_step: 0.03,
        max_angle_step: 6.0,
        frames: 8.0,
        stomp: -20.0,
        tip_fraction: 0.44,
        deck_mid_length: 0.59,
        deck_end_size: 0.13,
        deck_end_angle: 13.0,
        truck_z: -0.052,
        truck_y: -0.0565,
        lock_dist: super::ModeTuning::NORMAL.grind_lock_dist,
        lock_ledge: [0.35, 0.5, 0.65, 1.0],
        max_speed_sq_ledge: 36.0,
        max_speed_sq_rail: 64.0,
        max_speed_down: 7.0,
        tip_scalar: 0.7,
        adjust_max_angle: 11.0,
        offset: -0.2,
        landing_vel_scalar: 0.1,
        landing_max_angle: 20.0,
        score: 1_000_000.0,
        penalty: Curve::new(&[
            (0.0, -1000.0),
            (0.036_644_98, -764.286),
            (0.070_846_9, -610.714),
            (0.127_850_2, -428.571_4),
            (0.208_469_1, -278.571_4),
            (0.322_475_4, -142.857_1),
            (0.424_267_1, -50.000_01),
            (0.5, 0.0),
        ]),
        wipeout_xz: 6.0,
        wipeout_angle: 1.3,
        twist_smoothing: 0.15,
        twist_sensitivity: 0.3,
        twist_accel: 0.05,
        twist_step: 0.5,
        height_min: 0.6,
        height_max: 0.85,
        height_speed: 0.6,
    };

    /// The deck centre to a truck along the board (m): half the deck's middle plus the truck's offset.
    pub fn truck_distance(&self) -> f32 {
        self.deck_mid_length * 0.5 + self.truck_z
    }

    /// The five lock-on points in the board's frame (x right, y up, z forward): deck centre, nose tip,
    /// front truck, back truck, tail tip (SK8 grind_air_settings.rs).
    pub fn points(&self) -> [[f32; 3]; 5] {
        let half = self.deck_mid_length * 0.5;
        let tip = self.deck_end_size * self.tip_fraction;
        let y = libm::sinf(self.deck_end_angle * crate::math::PI / 180.0) * tip;
        let z = self.truck_distance();
        [
            [0.0, 0.0, 0.0],
            [0.0, y, half + tip],
            [0.0, self.truck_y, z],
            [0.0, self.truck_y, -z],
            [0.0, y, -(half + tip)],
        ]
    }

    /// Takes over a value from the rules pack (`class/key/field`). Returns whether it was one of these.
    pub(super) fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>) -> bool {
        macro_rules! scalar {
            ($field:expr) => {
                match f {
                    Some(f) => {
                        $field = f;
                        true
                    }
                    None => false,
                }
            };
        }
        macro_rules! curve {
            ($field:expr) => {
                match c {
                    Some(c) => {
                        $field = c;
                        true
                    }
                    None => false,
                }
            };
        }
        match name {
            "physics_grinds/default/FrictionVsTime" => curve!(self.friction_vs_time),
            "physics_grinds/default/DVEntryThreshScalarVsSinSlope" => curve!(self.entry_vs_slope),
            "physics_grinds/default/VertEngagementHelpVsVelY" => curve!(self.vert_help),
            "physics_grinds/default/TimeOfGravityReliefVsVerticalSpeed" => {
                curve!(self.relief_vs_vy)
            }
            "physics_grinds/default/GravityReliefTimeVsLinearSpeed" => {
                curve!(self.relief_vs_speed)
            }
            "physics_grinds/default/ExitLeanAngleVsTime" => curve!(self.exit_lean),
            "physics_grinds/default/ExitAssistVsLeanAngle" => curve!(self.exit_assist),
            "physics_grinds/default/PinVsSlope" => curve!(self.pin_vs_slope),
            "physics_grinds/default/TruckToWheel" => scalar!(self.truck_to_wheel),
            "physics_grinds/default/DeckCenterToTruck" => scalar!(self.deck_to_truck),
            "physics_grinds/default/TestDepthEpsilon" => scalar!(self.test_above),
            "physics_grinds/default/TestDepth" => scalar!(self.test_below),
            "physics_grinds_air/default/AngleRange50_50" => scalar!(self.angle_50),
            "physics_grinds_air/default/AngleRange5_0" => scalar!(self.angle_5o),
            "physics_grinds_air/default/AngleRangeBoardSlide" => scalar!(self.angle_board),
            "physics_grinds_air/default/AngleRangeTipSlide" => scalar!(self.angle_tip),
            "physics_grinds_air/default/AngleYAssist50_50" => scalar!(self.yaw_50),
            "physics_grinds_air/default/AngleYAssist5_0" => scalar!(self.yaw_5o),
            "physics_grinds_air/default/AngleYAssistBoardSlide" => scalar!(self.yaw_board),
            "physics_grinds_air/default/AngleYAssistTipSlide" => scalar!(self.yaw_tip),
            "physics_grinds_air/default/MaxDist50_50" => scalar!(self.reach_50),
            "physics_grinds_air/default/MaxDist5_0" => scalar!(self.reach_5o),
            "physics_grinds_air/default/MaxDistBoardSlide" => scalar!(self.reach_board),
            "physics_grinds_air/default/MaxDistTipSlide" => scalar!(self.reach_tip),
            "physics_grinds_air/default/MaxOffsetDist" => scalar!(self.max_offset),
            "physics_grinds_air/default/MaxOffsetDeltaPerFrame" => scalar!(self.max_offset_step),
            "physics_grinds_air/default/MaxAngleDeltaPerFrame" => scalar!(self.max_angle_step),
            // An integer in the database: a 4-byte one arrives as its bits read as a float.
            "physics_grinds_air/default/NumFramesToTest" => match f {
                Some(f) if f.to_bits() < 0x0080_0000 => {
                    self.frames = (f.to_bits() as f32).clamp(2.0, 12.0);
                    true
                }
                Some(f) if (2.0..=12.0).contains(&f) => {
                    self.frames = f;
                    true
                }
                _ => false,
            },
            "physics_grinds_air/default/StompAccelerationAdjust" => scalar!(self.stomp),
            "physics_grinds_air/default/TipOffsetFraction" => scalar!(self.tip_fraction),
            "physicsdeck/default/DeckMidLength" => scalar!(self.deck_mid_length),
            "physicsdeck/default/DeckFrontEndSize" => scalar!(self.deck_end_size),
            "physicsdeck/default/DeckFrontEndAngle" => scalar!(self.deck_end_angle),
            "physicstrucks/default/TruckZPosFront" => scalar!(self.truck_z),
            "physicstrucks/default/TruckYPos" => scalar!(self.truck_y),
            "physics_trajectory/default/GrindLockLedgeScalar2" => scalar!(self.lock_ledge[0]),
            "physics_trajectory/default/GrindLockLedgeScalar" => scalar!(self.lock_ledge[1]),
            "physics_trajectory/default/GrindLockLedgeLowSideScalar2" => {
                scalar!(self.lock_ledge[2])
            }
            "physics_trajectory/default/GrindLockLedgeLowSideScalar" => {
                scalar!(self.lock_ledge[3])
            }
            "physics_trajectory/default/GrindMaxSpeedSqrIntoLedge" => {
                scalar!(self.max_speed_sq_ledge)
            }
            "physics_trajectory/default/GrindMaxSpeedSqrIntoGrind" => {
                scalar!(self.max_speed_sq_rail)
            }
            "physics_trajectory/default/GrindMaxSpeedDownOntoGrind" => {
                scalar!(self.max_speed_down)
            }
            "physics_trajectory/default/GrindTipScalar" => scalar!(self.tip_scalar),
            "physics_trajectory/default/GrindAdjustMaxAngle" => scalar!(self.adjust_max_angle),
            "physics_trajectory/default/GrindOffset" => scalar!(self.offset),
            "physics_trajectory/default/GrindLandingVelScalar" => {
                scalar!(self.landing_vel_scalar)
            }
            "physics_trajectory/default/GrindLandingMaxAngle" => scalar!(self.landing_max_angle),
            "physics_trajectory/default/ScoreGrind" => scalar!(self.score),
            "physics_trajectory/default/GrindPenaltyVsDistToGrind" => curve!(self.penalty),
            "physics_wipeout/default/Wipeout_GrindXZDeck" => scalar!(self.wipeout_xz),
            "physics_wipeout/default/Wipeout_GrindMaxAngularDeckError" => {
                scalar!(self.wipeout_angle)
            }
            "anim_motion/grind_twist/twist_smoothing" => scalar!(self.twist_smoothing),
            "anim_motion/grind_twist/twist_sensitivity" => scalar!(self.twist_sensitivity),
            "anim_motion/grind_twist/twist_max_delta_delta" => scalar!(self.twist_accel),
            "anim_motion/grind_twist/twist_max_delta" => scalar!(self.twist_step),
            "anim_motion/grind_height/min_grind_disttocog" => scalar!(self.height_min),
            "anim_motion/grind_height/max_grind_disttocog" => scalar!(self.height_max),
            "anim_motion/grind_height/grind_disttocog_speed" => scalar!(self.height_speed),
            _ => false,
        }
    }
}

/// Where each grind value lives in Skate 3's tuning database (exported by `fg_import rules`).
pub const PATHS: &[&str] = &[
    "physics_grinds/default/FrictionVsTime",
    "physics_grinds/default/DVEntryThreshScalarVsSinSlope",
    "physics_grinds/default/VertEngagementHelpVsVelY",
    "physics_grinds/default/TimeOfGravityReliefVsVerticalSpeed",
    "physics_grinds/default/GravityReliefTimeVsLinearSpeed",
    "physics_grinds/default/ExitLeanAngleVsTime",
    "physics_grinds/default/ExitAssistVsLeanAngle",
    "physics_grinds/default/PinVsSlope",
    "physics_grinds/default/TruckToWheel",
    "physics_grinds/default/DeckCenterToTruck",
    "physics_grinds/default/TestDepthEpsilon",
    "physics_grinds/default/TestDepth",
    "physics_grinds_air/default/AngleRange50_50",
    "physics_grinds_air/default/AngleRange5_0",
    "physics_grinds_air/default/AngleRangeBoardSlide",
    "physics_grinds_air/default/AngleRangeTipSlide",
    "physics_grinds_air/default/AngleYAssist50_50",
    "physics_grinds_air/default/AngleYAssist5_0",
    "physics_grinds_air/default/AngleYAssistBoardSlide",
    "physics_grinds_air/default/AngleYAssistTipSlide",
    "physics_grinds_air/default/MaxDist50_50",
    "physics_grinds_air/default/MaxDist5_0",
    "physics_grinds_air/default/MaxDistBoardSlide",
    "physics_grinds_air/default/MaxDistTipSlide",
    "physics_grinds_air/default/MaxOffsetDist",
    "physics_grinds_air/default/MaxOffsetDeltaPerFrame",
    "physics_grinds_air/default/MaxAngleDeltaPerFrame",
    "physics_grinds_air/default/NumFramesToTest",
    "physics_grinds_air/default/StompAccelerationAdjust",
    "physics_grinds_air/default/TipOffsetFraction",
    "physicsdeck/default/DeckMidLength",
    "physicsdeck/default/DeckFrontEndSize",
    "physicsdeck/default/DeckFrontEndAngle",
    "physicstrucks/default/TruckZPosFront",
    "physicstrucks/default/TruckYPos",
    "physics_trajectory/default/GrindLockLedgeScalar2",
    "physics_trajectory/default/GrindLockLedgeScalar",
    "physics_trajectory/default/GrindLockLedgeLowSideScalar2",
    "physics_trajectory/default/GrindLockLedgeLowSideScalar",
    "physics_trajectory/default/GrindMaxSpeedSqrIntoLedge",
    "physics_trajectory/default/GrindMaxSpeedSqrIntoGrind",
    "physics_trajectory/default/GrindMaxSpeedDownOntoGrind",
    "physics_trajectory/default/GrindTipScalar",
    "physics_trajectory/default/GrindAdjustMaxAngle",
    "physics_trajectory/default/GrindOffset",
    "physics_trajectory/default/GrindLandingVelScalar",
    "physics_trajectory/default/GrindLandingMaxAngle",
    "physics_trajectory/default/ScoreGrind",
    "physics_trajectory/default/GrindPenaltyVsDistToGrind",
    "physics_wipeout/default/Wipeout_GrindXZDeck",
    "physics_wipeout/default/Wipeout_GrindMaxAngularDeckError",
    "anim_motion/grind_twist/twist_smoothing",
    "anim_motion/grind_twist/twist_sensitivity",
    "anim_motion/grind_twist/twist_max_delta_delta",
    "anim_motion/grind_twist/twist_max_delta",
    "anim_motion/grind_height/min_grind_disttocog",
    "anim_motion/grind_height/max_grind_disttocog",
    "anim_motion/grind_height/grind_disttocog_speed",
];
