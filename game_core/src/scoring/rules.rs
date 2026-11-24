//! The scoring module's numbers: Skate 3's line and combo timers (`Hash_349215E2E817703C/default`, SK8
//! `skate-data/src/scoring.rs` `ScoringData`) and the score collectors' curves and factors
//! (`Hash_546C36B656038E04/default`, SK8 `skate-data/src/scoring_fields.rs`, named here by the native
//! offset SK8's collectors read them at). The built-in values are Skate 3's stock ones (the session and
//! timer numbers are also those SK8's tests state; the curves were read off the disc's tuning database
//! with `fg_import values`). A rules pack carries the same paths in its `rules/tuning` entry.

use crate::tuning::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    // The session (Hash_349215E2E817703C).
    /// The line timer: points it loses a second (idle; scaled down while a trick goes on) and the most
    /// it holds. A line ends when it runs out.
    pub line_drain: f32,
    pub line_capacity: f32,
    /// The combo timer behind the multiplier: drain and capacity.
    pub combo_drain: f32,
    pub combo_capacity: f32,
    /// Combo timer points at or over which the multiplier is the level's (ascending).
    pub combo_levels: [(f32, f32); 3],
    /// A single reward over this refreshes the multiplier even downwards.
    pub combo_refresh: f32,
    /// A trick finished before it was announced (its completion delay) scores this share.
    pub unannounced_factor: f32,
    /// A bailed sequence scores this share (0: nothing).
    pub bail_factor: f32,
    /// A trick's repetition penalty against how often it was already done in the line.
    pub repetition: Curve,
    /// A trick's points to the factor its held rewards (grind, manual, grab hold) are scaled by.
    pub announcement: Curve,

    // The collectors (Hash_546C36B656038E04), by SK8's native offsets.
    /// 0x000 / 0x050: a powerslide's reward against its distance (m) and per second against its time.
    pub slide_distance: Curve,
    pub slide_time: Curve,
    /// 0x0f0 / 0x0a0: a manual's reward against distance and per second against time.
    pub manual_distance: Curve,
    pub manual_time: Curve,
    /// 0x2d0 / 0x280: a grind's reward against distance and per second against time.
    pub grind_distance: Curve,
    pub grind_time: Curve,
    /// 0x320: the air's reward against the height gained (m, landing below the take-off is negative).
    pub air_height_gain: Curve,
    /// 0x370: against the spin (degrees, whole half turns).
    pub air_spin: Curve,
    /// 0x3c0: against the distance flown across the ground (m).
    pub air_distance: Curve,
    /// 0x410: against the peak height over the take-off (m).
    pub air_peak: Curve,
    /// 0x460: a grab's reward per second against how long it was held.
    pub grab_hold: Curve,
    /// 0x5fc: seconds a ground sequence may hold the line timer from running out.
    pub ground_hold: f32,
    /// 0x604..0x618 and 0x664/0x668: the line and combo timers' drain while a trick goes on, per
    /// collector (off the board, on the ground, grinding, in the air).
    pub line_scale_offboard: f32,
    pub combo_scale_offboard: f32,
    pub line_scale_ground: f32,
    pub combo_scale_ground: f32,
    pub line_scale_grind: f32,
    pub combo_scale_grind: f32,
    pub line_scale_air: f32,
    pub combo_scale_air: f32,
    /// 0x61c / 0x62c / 0x628 / 0x630 / 0x620: a sequence's landing factor riding switch, fakie, onto a
    /// rail, clean, sketchy.
    pub land_switch: f32,
    pub land_fakie: f32,
    pub land_grind: f32,
    pub land_clean: f32,
    pub land_sketchy: f32,
    /// 0x640: points per body flip.
    pub body_flip: f32,
    /// 0x66c / 0x688: slower than this (m/s) at take-off, the air scores this share.
    pub stationary_speed: f32,
    pub stationary: f32,
    /// 0x684 / 0x694: the air's factor taken off switch, fakie.
    pub air_switch: f32,
    pub air_fakie: f32,
    /// 0x68c / 0x690: the air's factor out of a manual, off a rail.
    pub air_after_manual: f32,
    pub air_after_grind: f32,
}

const fn graph(x: [f32; 8], y: [f32; 8]) -> Curve {
    Curve::new(&[
        (x[0], y[0]),
        (x[1], y[1]),
        (x[2], y[2]),
        (x[3], y[3]),
        (x[4], y[4]),
        (x[5], y[5]),
        (x[6], y[6]),
        (x[7], y[7]),
    ])
}

impl Rules {
    pub const BUILT_IN: Rules = Rules {
        line_drain: 50.0,
        line_capacity: 400.0,
        combo_drain: 100.0,
        combo_capacity: 801.0,
        combo_levels: [(50.0, 1.5), (450.0, 2.0), (800.0, 3.0)],
        combo_refresh: 799.0,
        unannounced_factor: 0.3,
        bail_factor: 0.0,
        repetition: graph(
            [0.0, 1.0, 2.0, 3.0, 4.0, 10.0, 15.0, 127.0],
            [1.0, 0.5, 0.25, 0.125, 0.06125, 0.06125, 0.06125, 0.06125],
        ),
        announcement: graph(
            [20.0, 40.0, 60.0, 80.0, 100.0, 150.0, 520.0, 1000.0],
            [0.5, 1.0, 1.5, 2.0, 2.25, 2.5, 4.0, 5.0],
        ),
        slide_distance: graph(
            [
                0.0, 2.591743, 6.30734, 9.885322, 12.38532, 14.51835, 17.52294, 33.07339,
            ],
            [
                0.0, 62.5, 176.7857, 285.7143, 362.5, 421.4286, 510.7143, 1000.0,
            ],
        ),
        slide_time: graph(
            [
                0.03440367, 0.6674312, 1.548165, 2.305046, 3.619266, 4.266055, 5.298165, 6.0,
            ],
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.8928571, 0.0, 0.0],
        ),
        manual_distance: graph(
            [
                0.0, 115.1386, 279.3177, 501.0661, 635.3944, 770.7889, 895.5224, 1000.0,
            ],
            [
                0.0, 107.1429, 267.8572, 489.2858, 610.7142, 750.0, 875.0, 1000.0,
            ],
        ),
        manual_time: graph(
            [
                0.1, 0.100001, 2.534405, 4.34633, 5.940368, 7.763762, 9.162843, 10.0,
            ],
            [250.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ),
        grind_distance: graph(
            [1.0, 2.0, 6.0, 14.82225, 18.72964, 30.94463, 52.11726, 500.0],
            [
                0.0, 8.333335, 62.5, 135.7143, 159.8214, 181.25, 190.1786, 220.0,
            ],
        ),
        grind_time: graph(
            [
                1.0, 1.009174, 3.073395, 4.231651, 6.03211, 7.075688, 8.818808, 10.0,
            ],
            [40.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ),
        air_height_gain: graph(
            [-12.0, -8.379479, -5.0, -3.0, 0.75, 0.76, 5.785016, 12.0],
            [500.0, 319.6429, 150.0, 0.0, 0.0, 50.0, 246.4286, 500.0],
        ),
        air_spin: graph(
            [0.0, 180.0, 360.0, 540.0, 720.0, 900.0, 1080.0, 1260.0],
            [0.0, 180.0, 360.0, 540.0, 720.0, 900.0, 1080.0, 1260.0],
        ),
        air_distance: graph(
            [
                0.0, 7.0, 11.4, 16.88073, 21.69725, 26.6055, 30.73395, 34.44954,
            ],
            [
                0.0, 0.0, 75.0, 169.6429, 257.1429, 348.2143, 430.3571, 500.0,
            ],
        ),
        air_peak: graph(
            [0.0, 2.99, 3.0, 4.0, 6.0, 8.0, 10.0, 12.0],
            [0.0, 0.0, 50.0, 100.0, 200.0, 300.0, 400.0, 500.0],
        ),
        grab_hold: graph(
            [0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0],
            [0.0, 40.0, 75.0, 110.0, 160.0, 320.0, 550.0, 1000.0],
        ),
        ground_hold: 5.0,
        line_scale_offboard: 0.2,
        combo_scale_offboard: 0.2,
        line_scale_ground: 0.2,
        combo_scale_ground: 0.2,
        line_scale_grind: 0.05,
        combo_scale_grind: 0.1,
        line_scale_air: 0.5,
        combo_scale_air: 0.8,
        land_switch: 1.0,
        land_fakie: 1.0,
        land_grind: 1.1,
        land_clean: 1.1,
        land_sketchy: 0.9,
        body_flip: 300.0,
        stationary_speed: 4.0,
        stationary: 0.2,
        air_switch: 1.0,
        air_fakie: 1.25,
        air_after_manual: 1.1,
        air_after_grind: 1.1,
    };

    /// Takes over every value of these a `rules/tuning` entry carries (the others stay). Returns how
    /// many it used.
    pub fn apply(&mut self, data: &[u8]) -> usize {
        let mut used = 0;
        for (name, v) in crate::tuning::parse(data).unwrap_or_default() {
            if self.set(name, &v) {
                used += 1;
            }
        }
        used
    }

    /// Takes over one value (`class/key/field`). Returns whether it was one of these.
    pub fn set(&mut self, name: &str, v: &[f32]) -> bool {
        let f = v.first().copied().filter(|f| f.is_finite());
        let c = || Curve::from_values(v);
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
                match c() {
                    Some(c) => {
                        $field = c;
                        true
                    }
                    None => false,
                }
            };
        }
        if let Some(field) = name.strip_prefix(SESSION) {
            return match field {
                "Hash_FE02A45231F7B06A" => scalar!(self.line_drain),
                "Hash_57478337ACCFFF18" => scalar!(self.line_capacity),
                "Hash_88407506DC9780ED" => scalar!(self.combo_drain),
                "Hash_28767A5F961C7129" => scalar!(self.combo_capacity),
                "Hash_829887D8C24A5B8A" => scalar!(self.combo_levels[0].0),
                "Hash_4752056364FF91FE" => scalar!(self.combo_levels[0].1),
                "Hash_FB8408A15C17A9D4" => scalar!(self.combo_levels[1].0),
                "Hash_F4993A13ED7B44C1" => scalar!(self.combo_levels[1].1),
                "Hash_30AAE071A7868771" => scalar!(self.combo_levels[2].0),
                "Hash_DC0C3853F6C5FC2E" => scalar!(self.combo_levels[2].1),
                "Hash_E36197BFC8EA1CAD" => scalar!(self.combo_refresh),
                "Hash_2577DF0FCE5251E4" => scalar!(self.unannounced_factor),
                "Hash_B0B56FF046508506" => scalar!(self.bail_factor),
                "Hash_59D91EAABF033A24" => curve!(self.repetition),
                "Hash_263B277F8CA17126" => curve!(self.announcement),
                _ => false,
            };
        }
        let Some(field) = name.strip_prefix(COLLECTOR) else {
            return false;
        };
        match field {
            "Hash_8D544250D732B9A8" => curve!(self.slide_distance),
            "Hash_130B361B9E4D9369" => curve!(self.slide_time),
            "Hash_6DA2DFA9C8E3D863" => curve!(self.manual_distance),
            "Hash_156F53F56E861668" => curve!(self.manual_time),
            "Hash_E78EDEC321055E09" => curve!(self.grind_distance),
            "Hash_D7FBBD6E5E1EDF37" => curve!(self.grind_time),
            "Hash_8C24CF7B8227B348" => curve!(self.air_height_gain),
            "Hash_5A756D4B3A5036DD" => curve!(self.air_spin),
            "Hash_B552FB2712A897D6" => curve!(self.air_distance),
            "Hash_205C814AA3ACA8D4" => curve!(self.air_peak),
            "Hash_94F7B93068FE6FB1" => curve!(self.grab_hold),
            "Hash_BFDA1A507E2E55A1" => scalar!(self.ground_hold),
            "Hash_04A1775CB52A8E4D" => scalar!(self.combo_scale_offboard),
            "Hash_AD14550EFF3920DE" => scalar!(self.line_scale_offboard),
            "Hash_4BEA9C285A3CDA06" => scalar!(self.combo_scale_ground),
            "Hash_F813B960D0450300" => scalar!(self.line_scale_ground),
            "Hash_391821526C05A7F1" => scalar!(self.combo_scale_grind),
            "Hash_B3C69AAF221F34F9" => scalar!(self.line_scale_grind),
            "Hash_BC91E9C84E099F2C" => scalar!(self.combo_scale_air),
            "Hash_C33E4AA507A2CB4C" => scalar!(self.line_scale_air),
            "Hash_E2AFE1B06292209D" => scalar!(self.land_switch),
            "Hash_62B9C3DAA846E240" => scalar!(self.land_sketchy),
            "Hash_77DD0728B4736957" => scalar!(self.land_grind),
            "Hash_05BBAF2E866E083E" => scalar!(self.land_fakie),
            "Hash_5A2D1476F08690EC" => scalar!(self.land_clean),
            "Hash_694410FC8D57E794" => scalar!(self.body_flip),
            "Hash_AB8EACF72B291746" => scalar!(self.stationary_speed),
            "Hash_EE42BA52A1313FDB" => scalar!(self.air_switch),
            "Hash_945369EBD1E9A062" => scalar!(self.stationary),
            "Hash_123E054E8895CAFE" => scalar!(self.air_after_manual),
            "Hash_63E61D37475E6AB9" => scalar!(self.air_after_grind),
            "Hash_AC54DE5C98B5DFAD" => scalar!(self.air_fakie),
            _ => false,
        }
    }
}

const SESSION: &str = "Hash_349215E2E817703C/default/";
const COLLECTOR: &str = "Hash_546C36B656038E04/default/";

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "Hash_349215E2E817703C/default/Hash_FE02A45231F7B06A",
    "Hash_349215E2E817703C/default/Hash_57478337ACCFFF18",
    "Hash_349215E2E817703C/default/Hash_88407506DC9780ED",
    "Hash_349215E2E817703C/default/Hash_28767A5F961C7129",
    "Hash_349215E2E817703C/default/Hash_829887D8C24A5B8A",
    "Hash_349215E2E817703C/default/Hash_4752056364FF91FE",
    "Hash_349215E2E817703C/default/Hash_FB8408A15C17A9D4",
    "Hash_349215E2E817703C/default/Hash_F4993A13ED7B44C1",
    "Hash_349215E2E817703C/default/Hash_30AAE071A7868771",
    "Hash_349215E2E817703C/default/Hash_DC0C3853F6C5FC2E",
    "Hash_349215E2E817703C/default/Hash_E36197BFC8EA1CAD",
    "Hash_349215E2E817703C/default/Hash_2577DF0FCE5251E4",
    "Hash_349215E2E817703C/default/Hash_B0B56FF046508506",
    "Hash_349215E2E817703C/default/Hash_59D91EAABF033A24",
    "Hash_349215E2E817703C/default/Hash_263B277F8CA17126",
    "Hash_546C36B656038E04/default/Hash_8D544250D732B9A8",
    "Hash_546C36B656038E04/default/Hash_130B361B9E4D9369",
    "Hash_546C36B656038E04/default/Hash_6DA2DFA9C8E3D863",
    "Hash_546C36B656038E04/default/Hash_156F53F56E861668",
    "Hash_546C36B656038E04/default/Hash_E78EDEC321055E09",
    "Hash_546C36B656038E04/default/Hash_D7FBBD6E5E1EDF37",
    "Hash_546C36B656038E04/default/Hash_8C24CF7B8227B348",
    "Hash_546C36B656038E04/default/Hash_5A756D4B3A5036DD",
    "Hash_546C36B656038E04/default/Hash_B552FB2712A897D6",
    "Hash_546C36B656038E04/default/Hash_205C814AA3ACA8D4",
    "Hash_546C36B656038E04/default/Hash_94F7B93068FE6FB1",
    "Hash_546C36B656038E04/default/Hash_BFDA1A507E2E55A1",
    "Hash_546C36B656038E04/default/Hash_04A1775CB52A8E4D",
    "Hash_546C36B656038E04/default/Hash_AD14550EFF3920DE",
    "Hash_546C36B656038E04/default/Hash_4BEA9C285A3CDA06",
    "Hash_546C36B656038E04/default/Hash_F813B960D0450300",
    "Hash_546C36B656038E04/default/Hash_391821526C05A7F1",
    "Hash_546C36B656038E04/default/Hash_B3C69AAF221F34F9",
    "Hash_546C36B656038E04/default/Hash_BC91E9C84E099F2C",
    "Hash_546C36B656038E04/default/Hash_C33E4AA507A2CB4C",
    "Hash_546C36B656038E04/default/Hash_E2AFE1B06292209D",
    "Hash_546C36B656038E04/default/Hash_62B9C3DAA846E240",
    "Hash_546C36B656038E04/default/Hash_77DD0728B4736957",
    "Hash_546C36B656038E04/default/Hash_05BBAF2E866E083E",
    "Hash_546C36B656038E04/default/Hash_5A2D1476F08690EC",
    "Hash_546C36B656038E04/default/Hash_694410FC8D57E794",
    "Hash_546C36B656038E04/default/Hash_AB8EACF72B291746",
    "Hash_546C36B656038E04/default/Hash_EE42BA52A1313FDB",
    "Hash_546C36B656038E04/default/Hash_945369EBD1E9A062",
    "Hash_546C36B656038E04/default/Hash_123E054E8895CAFE",
    "Hash_546C36B656038E04/default/Hash_63E61D37475E6AB9",
    "Hash_546C36B656038E04/default/Hash_AC54DE5C98B5DFAD",
];

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn every_path_is_one_of_these_and_the_rules_entry_sets_them() {
        let mut r = Rules::BUILT_IN;
        for p in PATHS {
            assert!(r.set(p, &[1.0]) || r.set(p, &[0.0, 1.0, 2.0, 3.0]), "{p}");
        }
        let mut r = Rules::BUILT_IN;
        let data = crate::tuning::encode(&[
            (
                "Hash_349215E2E817703C/default/Hash_57478337ACCFFF18",
                vec![200.0],
            ),
            (
                "Hash_349215E2E817703C/default/Hash_59D91EAABF033A24",
                vec![0.0, 1.0, 1.0, 0.25],
            ),
            (
                "Hash_546C36B656038E04/default/Hash_5A2D1476F08690EC",
                vec![1.5],
            ),
            ("physics_push/default/MaxPushableSpeed", vec![9.0]),
        ]);
        assert_eq!(r.apply(&data), 3);
        assert_eq!(r.line_capacity, 200.0);
        assert_eq!(r.repetition.eval(1.0), 0.25);
        assert_eq!(r.land_clean, 1.5);
        assert_eq!(Rules::BUILT_IN.repetition.eval(2.0), 0.25, "Skate 3's");
    }
}
