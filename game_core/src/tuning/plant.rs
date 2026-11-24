//! Foot plant tuning: the numbers Skate 3's foot plants read from its tuning database. The FootJump
//! launch of a boneless, fastplant or no-comply (SK8 `skate-game/src/physics/boneless.rs`, physical state
//! 602: four point graphs in `Hash_CCB95A83C78B4FF9/default`) and the air foot plant's manager
//! (`physics_footplantmanager/default`, SK8 `physics/footplant/settings.rs`). The built-in values are
//! Skate 3's stock ones (read off the disc's tuning database with `fg_import values`); the rules pack
//! carries the same paths.

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlantTuning {
    // The FootJump launch (Hash_CCB95A83C78B4FF9).
    /// The launch's speed along the ground's up (m/s) against the ground normal's y, times a share
    /// against the speed along the ground.
    pub jump_up_vs_normal_y: Curve,
    pub jump_up_vs_speed: Curve,
    /// The speed along the ground kept (a factor) against that speed.
    pub jump_speed_scale: Curve,
    /// How much of the launch is the planted one (the rest keeps the velocity across the ground and
    /// only sets its up speed), against the ground normal's y.
    pub jump_blend_vs_normal_y: Curve,

    // physics_footplantmanager.
    /// The deck's half extents (m, x across, y up, z along) a toe outside of is free to plant, and how
    /// far up the box is moved.
    pub deck_bb: [f32; 3],
    pub deck_bb_y_offset: f32,
    /// The most the leg may be off its planting direction (degrees).
    pub max_leg_angle_error: f32,
    /// How far from the centre of mass the planting foot reaches (m).
    pub leg_length: f32,
    /// The planted foot's volume over the contact (m).
    pub foot_volume_y_offset: f32,
    /// The swing's rate (a factor) against the speed along the leg (m/s).
    pub radial_speed_scale: Curve,
    /// Too fast across or down the surface while planted wipes out (m/s).
    pub max_horizontal_speed: f32,
    pub max_descending_speed: f32,
    /// The release's speed out along the leg (m/s).
    pub release_outward_speed: f32,
    /// The swing's curve: its start and end handles (shares of the swing's time x its speeds), the end
    /// angle from straight up over the foot (degrees) and the leg's length there (m).
    pub start_handle: f32,
    pub end_handle: f32,
    pub end_angle: f32,
    pub end_leg_length: f32,
    /// The swing's shortest and longest time (s).
    pub min_duration: f32,
    pub max_duration: f32,
}

impl PlantTuning {
    /// Skate 3's stock values (`Hash_CCB95A83C78B4FF9/default`, `physics_footplantmanager/default`).
    pub const BUILT_IN: PlantTuning = PlantTuning {
        jump_up_vs_normal_y: Curve::new(&[
            (0.0, 1.028_571),
            (0.092_833_88, 1.178_571),
            (0.216_612_4, 1.435_714),
            (0.358_306_2, 1.885_714),
            (0.534_202, 2.410_714),
            (0.693_811_1, 2.689_286),
            (0.851_791_6, 3.0),
            (1.0, 3.0),
        ]),
        jump_up_vs_speed: Curve::new(&[
            (0.0, 1.0),
            (0.110_749_2, 1.0),
            (0.229_641_7, 1.0),
            (0.363_192_2, 1.0),
            (0.516_286_7, 1.0),
            (0.677_524_4, 1.0),
            (0.837_133_5, 1.0),
            (0.998_371_4, 1.0),
        ]),
        jump_speed_scale: Curve::new(&[
            (0.0, 2.0),
            (1.726_384, 1.314_286),
            (3.583_062, 0.914_285_7),
            (8.110_749, 0.785_714_3),
            (15.342_02, 0.778_571_4),
            (17.459_28, 0.721_428_6),
            (19.055_37, 0.542_857_2),
            (20.0, 0.521_428_6),
        ]),
        jump_blend_vs_normal_y: Curve::new(&[
            (0.0, 0.746_428_5),
            (0.096_091_2, 0.767_857_1),
            (0.353_420_2, 0.803_571_4),
            (0.503_257_3, 0.817_857_1),
            (0.679_153_1, 0.857_142_9),
            (0.845_276_9, 0.95),
            (0.910_423_5, 1.0),
            (1.0, 1.0),
        ]),
        deck_bb: [0.23, 0.06, 0.4],
        deck_bb_y_offset: 0.04,
        max_leg_angle_error: 30.0,
        leg_length: 0.92,
        foot_volume_y_offset: 0.02,
        radial_speed_scale: Curve::new(&[
            (0.0, 0.951_785_7),
            (1.872_964, 0.841_071_5),
            (2.931_596, 0.785_714_4),
            (4.234_528, 0.742_857_1),
            (5.798_045, 0.710_714_3),
            (7.263_844, 0.692_857),
            (8.876_222, 0.689_285_6),
            (10.0, 0.685_714_4),
        ]),
        max_horizontal_speed: 11.0,
        max_descending_speed: 10.0,
        release_outward_speed: 2.0,
        start_handle: 0.3,
        end_handle: 0.3,
        end_angle: 0.0,
        end_leg_length: 0.88,
        min_duration: 0.08,
        max_duration: 0.25,
    };

    /// Takes over a value from the rules pack (`class/key/field`). Returns whether it was one of these.
    pub(super) fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>, v: &[f32]) -> bool {
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
        const JUMP: &str = "Hash_CCB95A83C78B4FF9/default/";
        const FP: &str = "physics_footplantmanager/default/";
        if let Some(field) = name.strip_prefix(JUMP) {
            return match field {
                "Hash_6D781EFAF01E707D" => curve!(self.jump_up_vs_normal_y),
                "Hash_C9112CCD0BCB1850" => curve!(self.jump_up_vs_speed),
                "Hash_88D0CDEFA38A36D6" => curve!(self.jump_speed_scale),
                "Hash_9228B7F18C223F9C" => curve!(self.jump_blend_vs_normal_y),
                _ => false,
            };
        }
        let Some(field) = name.strip_prefix(FP) else {
            return false;
        };
        match field {
            "DeckBB" if v.len() >= 3 && v[..3].iter().all(|x| x.is_finite()) => {
                self.deck_bb = [v[0], v[1], v[2]];
                true
            }
            "DeckBBYOffset" => scalar!(self.deck_bb_y_offset),
            "MaxLegAngleError" => scalar!(self.max_leg_angle_error),
            "LegLengthOnLanding" => scalar!(self.leg_length),
            "FootVolumeYOffset" => scalar!(self.foot_volume_y_offset),
            "Hash_8A8887218466ED15" => curve!(self.radial_speed_scale),
            "Hash_149A5D0D4768D2B1" => scalar!(self.max_horizontal_speed),
            "Hash_52436D9F4FE56BCB" => scalar!(self.max_descending_speed),
            "Hash_C5F92168DFAFD0B8" => scalar!(self.release_outward_speed),
            "Hash_628C228DCC7B6ED5" => scalar!(self.end_handle),
            "Hash_4F45DA86685CF029" => scalar!(self.end_angle),
            "Hash_0DE0027B9913C5D5" => scalar!(self.end_leg_length),
            "Hash_5C4341EF35F4B2B0" => scalar!(self.start_handle),
            "Hash_D6224B710A36D643" => scalar!(self.min_duration),
            "Hash_68C05A7FCC6BB718" => scalar!(self.max_duration),
            _ => false,
        }
    }
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "Hash_CCB95A83C78B4FF9/default/Hash_6D781EFAF01E707D",
    "Hash_CCB95A83C78B4FF9/default/Hash_C9112CCD0BCB1850",
    "Hash_CCB95A83C78B4FF9/default/Hash_88D0CDEFA38A36D6",
    "Hash_CCB95A83C78B4FF9/default/Hash_9228B7F18C223F9C",
    "physics_footplantmanager/default/DeckBB",
    "physics_footplantmanager/default/DeckBBYOffset",
    "physics_footplantmanager/default/MaxLegAngleError",
    "physics_footplantmanager/default/LegLengthOnLanding",
    "physics_footplantmanager/default/FootVolumeYOffset",
    "physics_footplantmanager/default/Hash_8A8887218466ED15",
    "physics_footplantmanager/default/Hash_149A5D0D4768D2B1",
    "physics_footplantmanager/default/Hash_52436D9F4FE56BCB",
    "physics_footplantmanager/default/Hash_C5F92168DFAFD0B8",
    "physics_footplantmanager/default/Hash_628C228DCC7B6ED5",
    "physics_footplantmanager/default/Hash_4F45DA86685CF029",
    "physics_footplantmanager/default/Hash_0DE0027B9913C5D5",
    "physics_footplantmanager/default/Hash_5C4341EF35F4B2B0",
    "physics_footplantmanager/default/Hash_D6224B710A36D643",
    "physics_footplantmanager/default/Hash_68C05A7FCC6BB718",
];
