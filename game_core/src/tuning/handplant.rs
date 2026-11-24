//! Hand plant tuning: the numbers Skate 3's hand plants (inverts) read from its tuning database. The
//! coping search's velocity window (`Hash_55B5940075F78842/default`, SK8 `physics/handplant/settings.rs`,
//! cached layouts `55B5940075F78842` and `0F38084037CB9175`), the hand plant manager
//! (`physics_handplantmanager/default`) and the clips' thresholds (`anim_handplant/default`). The built-in
//! values are Skate 3's stock ones (read off the disc's tuning database with `fg_import values`); the
//! rules pack carries the same paths.

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandPlantTuning {
    /// The window the flight's apex is looked for in (SK8 contact.rs `window`), each against the speed
    /// up the ramp's plane: the lowest height, the least and most sideways speed, how far above and
    /// below the flight's own height, how far sideways either way (m, m/s).
    pub window: [Curve; 7],
    /// How far into the ramp a coping is looked for (m), and how far under the apex the hand reaches.
    pub depth: f32,
    pub window_drop: f32,
    /// The hand plant's clock against the time from its apex (s): it slows over the apex (the hang).
    pub time_warp: Curve,
    /// How far round the heading is turned out of it, into it and out of it (shares against shares of
    /// the time).
    pub out_heading: Curve,
    pub into_rotation: Curve,
    pub out_rotation: Curve,
    /// The hand's reach against the share of the plant (m).
    pub hand_radius: Curve,
    /// Half the time the path round the apex takes (s), and how long the start blends from the flight
    /// it came in on.
    pub curve_half_time: f32,
    pub entry_blend: f32,
    /// The hand's timings (s): out, into, release; and how far it reaches before the coping.
    pub hand_out: f32,
    pub hand_into: f32,
    pub hand_release: f32,
    pub hand_approach: f32,
    /// Only faster than this (m/s), up a ramp steeper than this (degrees).
    pub min_speed: f32,
    pub min_slope: f32,
    /// The rotation's time (s); committed once the apex is nearer than this (s).
    pub rotation_time: f32,
    pub committed_time: f32,
    /// The least speed back over the ramp out of it (m/s).
    pub min_out_speed: f32,
    /// Ticks a change of side has to last before the side hint is dropped.
    pub direction_frames: u32,
    /// The apex: this far from the coping (m), this far round from straight above it (rad, towards the
    /// ramp).
    pub apex_radius: f32,
    pub apex_angle: f32,
    /// The clips' out, into and antic thresholds (s to the apex: `ShouldPlayHandPlantAnim`).
    pub anim: [f32; 3],
}

const fn flat(x: [f32; 4], y: f32) -> Curve {
    Curve::new(&[(x[0], y), (x[1], y), (x[2], y), (x[3], y)])
}

impl HandPlantTuning {
    /// Skate 3's stock values.
    pub const BUILT_IN: HandPlantTuning = HandPlantTuning {
        window: [
            flat([0.0, 3.0, 5.472_313, 7.0], 3.5),
            flat([0.0, 1.75, 3.5, 7.0], 0.5),
            flat([0.0, 2.667_752, 4.856_678, 7.0], 1.0),
            flat([0.0, 1.858_306, 4.252_443, 7.0], 4.8125),
            flat([0.0, 2.428_339, 4.799_674, 7.0], 0.799_999_7),
            flat([0.0, 1.778_502, 3.522_801, 7.0], 2.8),
            flat([0.0, 1.75, 3.5, 7.0], 1.333_333),
        ],
        depth: 1.5,
        window_drop: 1.0,
        time_warp: Curve::new(&[
            (-1.0, 0.0),
            (-0.456_026_1, 0.0),
            (-0.332_247_6, -0.057_142_85),
            (-0.133_550_5, -0.242_857_2),
            (-0.003_257_334, -0.642_857_1),
            (0.107_491_9, -0.307_142_9),
            (0.293_159_6, -0.064_285_7),
            (0.508_143_3, 0.0),
        ]),
        out_heading: Curve::new(&[
            (0.0, 0.0),
            (0.197_068_4, 0.028_571_43),
            (0.469_055_4, 0.128_571_4),
            (0.620_521_2, 0.267_857_1),
            (0.744_299_7, 0.453_571_4),
            (0.845_276_9, 0.653_571_4),
            (0.938_110_8, 0.871_428_5),
            (1.0, 1.0),
        ]),
        into_rotation: Curve::new(&[
            (0.0, 0.0),
            (0.097_719_87, 0.253_571_4),
            (0.221_498_4, 0.482_142_9),
            (0.382_736_1, 0.7),
            (0.548_86, 0.807_142_9),
            (0.690_553_7, 0.885_714_3),
            (0.817_589_6, 0.935_714_3),
            (1.0, 1.0),
        ]),
        out_rotation: Curve::new(&[
            (0.0, 0.0),
            (0.157_980_5, 0.196_428_6),
            (0.328_990_2, 0.35),
            (0.485_342, 0.475),
            (0.643_322_5, 0.585_714_3),
            (0.793_159_6, 0.767_857_1),
            (0.881_107_5, 0.882_142_8),
            (1.0, 1.0),
        ]),
        hand_radius: Curve::new(&[
            (0.0, 0.0),
            (0.364_820_8, 0.039_062_5),
            (0.742_671, 0.242_187_5),
            (1.0, 0.75),
        ]),
        curve_half_time: 0.15,
        entry_blend: 0.12,
        hand_out: 0.35,
        hand_into: 0.1,
        hand_release: 0.25,
        hand_approach: 0.02,
        min_speed: 0.5,
        min_slope: 45.0,
        rotation_time: 0.5,
        committed_time: 0.55,
        min_out_speed: 0.9,
        direction_frames: 60,
        apex_radius: 0.8,
        apex_angle: -0.3,
        anim: [0.1, 0.53, 0.75],
    };

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
        if let Some(field) = name.strip_prefix(WINDOW) {
            return match field {
                "Hash_FF0DE8A42210E8C0" => curve!(self.window[0]),
                "Hash_B90C78786ED04A7D" => curve!(self.window[1]),
                "Hash_DC7947C6DD03CDAA" => curve!(self.window[2]),
                "Hash_0ACB7AAE2662FBB2" => curve!(self.window[3]),
                "Hash_CF7F668AB995380D" => curve!(self.window[4]),
                "Hash_8348DA199CC87B7A" => curve!(self.window[5]),
                "Hash_F27F622CAE33BD50" => curve!(self.window[6]),
                "Hash_3D9138A04C6D43F6" => scalar!(self.depth),
                "Hash_A046D0516C0BD62B" => scalar!(self.window_drop),
                _ => false,
            };
        }
        if let Some(field) = name.strip_prefix(ANIM) {
            return match field {
                "Hash_B9012B86EB5258FE" => scalar!(self.anim[0]),
                "Hash_512342FB1D4A7D57" => scalar!(self.anim[1]),
                "Hash_673F3B1BD056C39E" => scalar!(self.anim[2]),
                _ => false,
            };
        }
        let Some(field) = name.strip_prefix(MANAGER) else {
            return false;
        };
        match field {
            "Hash_F551DF3289124FB0" => curve!(self.time_warp),
            "Hash_8FC9A4F9002CF9E0" => curve!(self.out_heading),
            "Hash_F61B0FB73C39311B" => curve!(self.into_rotation),
            "Hash_D4EF764AF4D5F1CA" => curve!(self.out_rotation),
            "Hash_5494E3F9BE572D6B" => curve!(self.hand_radius),
            "Hash_2460D6544C3B64B7" => scalar!(self.curve_half_time),
            "Hash_CCD7A82CE1BBFFF2" => scalar!(self.entry_blend),
            "Hash_E0895C189D77CE16" => scalar!(self.hand_out),
            "Hash_01EB855A0ACBD84C" => scalar!(self.hand_into),
            "Hash_9C02B9E6998D1321" => scalar!(self.hand_release),
            "Hash_7C8B87F7435221C4" => scalar!(self.hand_approach),
            "Hash_63303CA6C611DC79" => scalar!(self.min_speed),
            "Hash_D1622917F6DD215F" => scalar!(self.min_slope),
            "Hash_D595A32C5F02178F" => scalar!(self.rotation_time),
            "Hash_B5EC5A0941B407A4" => scalar!(self.committed_time),
            "Hash_49C91803479B9BED" => scalar!(self.min_out_speed),
            "Hash_A046D0516C0BD62B" => scalar!(self.apex_radius),
            "Hash_90CDD541098AB4B5" => scalar!(self.apex_angle),
            _ => false,
        }
    }
}

const WINDOW: &str = "Hash_55B5940075F78842/default/";
const MANAGER: &str = "physics_handplantmanager/default/";
const ANIM: &str = "anim_handplant/default/";

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "Hash_55B5940075F78842/default/Hash_FF0DE8A42210E8C0",
    "Hash_55B5940075F78842/default/Hash_B90C78786ED04A7D",
    "Hash_55B5940075F78842/default/Hash_DC7947C6DD03CDAA",
    "Hash_55B5940075F78842/default/Hash_0ACB7AAE2662FBB2",
    "Hash_55B5940075F78842/default/Hash_CF7F668AB995380D",
    "Hash_55B5940075F78842/default/Hash_8348DA199CC87B7A",
    "Hash_55B5940075F78842/default/Hash_F27F622CAE33BD50",
    "Hash_55B5940075F78842/default/Hash_3D9138A04C6D43F6",
    "Hash_55B5940075F78842/default/Hash_A046D0516C0BD62B",
    "physics_handplantmanager/default/Hash_F551DF3289124FB0",
    "physics_handplantmanager/default/Hash_8FC9A4F9002CF9E0",
    "physics_handplantmanager/default/Hash_F61B0FB73C39311B",
    "physics_handplantmanager/default/Hash_D4EF764AF4D5F1CA",
    "physics_handplantmanager/default/Hash_5494E3F9BE572D6B",
    "physics_handplantmanager/default/Hash_2460D6544C3B64B7",
    "physics_handplantmanager/default/Hash_CCD7A82CE1BBFFF2",
    "physics_handplantmanager/default/Hash_E0895C189D77CE16",
    "physics_handplantmanager/default/Hash_01EB855A0ACBD84C",
    "physics_handplantmanager/default/Hash_9C02B9E6998D1321",
    "physics_handplantmanager/default/Hash_7C8B87F7435221C4",
    "physics_handplantmanager/default/Hash_63303CA6C611DC79",
    "physics_handplantmanager/default/Hash_D1622917F6DD215F",
    "physics_handplantmanager/default/Hash_D595A32C5F02178F",
    "physics_handplantmanager/default/Hash_B5EC5A0941B407A4",
    "physics_handplantmanager/default/Hash_49C91803479B9BED",
    "physics_handplantmanager/default/Hash_A046D0516C0BD62B",
    "physics_handplantmanager/default/Hash_90CDD541098AB4B5",
    "anim_handplant/default/Hash_B9012B86EB5258FE",
    "anim_handplant/default/Hash_512342FB1D4A7D57",
    "anim_handplant/default/Hash_673F3B1BD056C39E",
];
