//! Skate 3's difficulty: the Game Settings "Play Mode" row (Easy, Normal, Hardcore) and the
//! physics mode collection it selects (`physics_mode/easy`, `/normal`, `/hardcore`).
//!
//! Skate 3 doesn't scale anything by difficulty: each mode is its own collection of values in the tuning
//! database, and the selected one is the one the physics reads (SK8 `skate-game/src/difficulty.rs`:
//! "host preference selects the native physics_mode collection, never a multiplier"). The retail row
//! (`ID_GAMESETTINGS_DIFFICULTY_OPTIONS`, settings +180) steps `v + dir` and wraps over 0..2, and the
//! change applies at once (SK8 docs/hails-additions/31-retail-menus.md, `sub_8260DE60`).
//!
//! [`ModeTuning`] holds the fields of the collection flatground reads; the rules pack carries them for
//! all three modes ([`PATHS`]) and [`crate::tuning::Tuning::set_difficulty`] puts the selected mode's
//! into the riding tuning. The built-in values are Skate 3's (read from the disc's tuning database).

/// Skate 3's difficulty (its physics mode). Normal is Skate 3's default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hardcore,
}

impl Difficulty {
    /// In the retail row's order (its values 0, 1, 2).
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hardcore];

    /// The physics mode collection's name, as the settings file writes it too.
    pub fn key(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hardcore => "hardcore",
        }
    }

    pub fn parse(key: &str) -> Option<Difficulty> {
        Difficulty::ALL.into_iter().find(|d| d.key() == key)
    }

    /// The retail row's value (0 Easy, 1 Normal, 2 Hardcore).
    pub fn index(self) -> usize {
        self as usize
    }

    /// Left (-1) or right (+1) on the row: `v + dir`, above 2 back to 0, below 0 to 2.
    pub fn step(self, dir: i8) -> Difficulty {
        let n = Difficulty::ALL.len() as i32;
        Difficulty::ALL[(self.index() as i32 + dir as i32).rem_euclid(n) as usize]
    }
}

/// The physics mode's values flatground reads (one collection per difficulty).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeTuning {
    /// Most speed one tick of push contact adds, at a standstill and at top push speed (m/s;
    /// `MaxPushDVStart`, `MaxPushDVEnd`): `Tuning::push_tick_limit_slow` / `_fast`.
    pub push_dv_start: f32,
    pub push_dv_end: f32,
    /// Pumping (`PumpEffectFactor`, `Hash_D77AFD320B6241C5`, `UnintentionalPumpScalar`):
    /// `Tuning::pump_factor`, `pump_max_accel`, `pump_unintentional`.
    pub pump_factor: f32,
    pub pump_max_accel: f32,
    pub pump_unintentional: f32,
    /// The regular jump's height range (m; `JumpMinHeight`, `JumpMaxHeight`): `Tuning::jump_low` /
    /// `jump_high`.
    pub jump_low: f32,
    pub jump_high: f32,
    /// Air grind targeting distance (m, `GrindLockDist`): `GrindTuning::lock_dist`.
    pub grind_lock_dist: f32,
    /// The landing assist's body spin limit (rad/s, `MaxAutoBodySpinSpeed`): `Tuning::max_auto_spin`.
    pub max_auto_spin: f32,
    /// Speed wobble onset (m/s, `Hash_77AFCE78FE1206CA`) and strength (`Hash_5B57F2CCCCEEF430`):
    /// `WobbleTuning::activation` / `amplitude_scale`.
    pub wobble_onset: f32,
    pub wobble_amplitude: f32,
    /// The manual's corrective force (`Hash_F8CBC0F5FEF2240E`) and its least speed (m/s,
    /// `Hash_84CB2F459F3FC811`): `Tuning::manual_corrective` / `manual_correction_speed`. Without the
    /// force a drag against the manual's balance ends it (`PhysicsWantsManualExit`).
    pub manual_corrective: bool,
    pub manual_correction_speed: f32,
    /// Ground-impact bail threshold (`Wipeout_GroundXZAcceleration`): `Tuning::wipe_ground_closing`.
    pub wipe_ground_closing: f32,
}

/// The collection's field names, in [`ModeTuning`]'s order.
pub const FIELDS: [&str; 14] = [
    "MaxPushDVStart",
    "MaxPushDVEnd",
    "PumpEffectFactor",
    "Hash_D77AFD320B6241C5",
    "UnintentionalPumpScalar",
    "JumpMinHeight",
    "JumpMaxHeight",
    "GrindLockDist",
    "MaxAutoBodySpinSpeed",
    "Hash_77AFCE78FE1206CA",
    "Hash_5B57F2CCCCEEF430",
    "Hash_F8CBC0F5FEF2240E",
    "Hash_84CB2F459F3FC811",
    "Wipeout_GroundXZAcceleration",
];

macro_rules! mode_paths {
    ($m:literal) => {
        [
            concat!("physics_mode/", $m, "/MaxPushDVStart"),
            concat!("physics_mode/", $m, "/MaxPushDVEnd"),
            concat!("physics_mode/", $m, "/PumpEffectFactor"),
            concat!("physics_mode/", $m, "/Hash_D77AFD320B6241C5"),
            concat!("physics_mode/", $m, "/UnintentionalPumpScalar"),
            concat!("physics_mode/", $m, "/JumpMinHeight"),
            concat!("physics_mode/", $m, "/JumpMaxHeight"),
            concat!("physics_mode/", $m, "/GrindLockDist"),
            concat!("physics_mode/", $m, "/MaxAutoBodySpinSpeed"),
            concat!("physics_mode/", $m, "/Hash_77AFCE78FE1206CA"),
            concat!("physics_mode/", $m, "/Hash_5B57F2CCCCEEF430"),
            concat!("physics_mode/", $m, "/Hash_F8CBC0F5FEF2240E"),
            concat!("physics_mode/", $m, "/Hash_84CB2F459F3FC811"),
            concat!("physics_mode/", $m, "/Wipeout_GroundXZAcceleration"),
        ]
    };
}

/// Where each mode's values live in Skate 3's tuning database, per [`Difficulty`] in [`FIELDS`] order.
pub const PATHS: [[&str; FIELDS.len()]; 3] = [
    mode_paths!("easy"),
    mode_paths!("normal"),
    mode_paths!("hardcore"),
];

impl ModeTuning {
    /// Skate 3's `physics_mode/easy`: stronger pushes, higher low jumps, a wider grind lock, a later
    /// and softer speed wobble, and no manual corrective force.
    pub const EASY: ModeTuning = ModeTuning {
        push_dv_start: 1.5,
        push_dv_end: 1.5,
        pump_factor: 18.0,
        pump_max_accel: 10.0,
        pump_unintentional: 1.0,
        jump_low: 1.71,
        jump_high: 1.71,
        grind_lock_dist: 1.35,
        max_auto_spin: 8.0,
        wobble_onset: 22.0,
        wobble_amplitude: 0.25,
        manual_corrective: false,
        manual_correction_speed: 4.0,
        wipe_ground_closing: 9.0,
    };

    /// Skate 3's `physics_mode/normal` (the default).
    pub const NORMAL: ModeTuning = ModeTuning {
        push_dv_start: 0.6,
        push_dv_end: 0.5,
        pump_factor: 18.0,
        pump_max_accel: 10.0,
        pump_unintentional: 0.7,
        jump_low: 1.33,
        jump_high: 1.71,
        grind_lock_dist: 0.9,
        max_auto_spin: 8.0,
        wobble_onset: 15.0,
        wobble_amplitude: 0.5,
        manual_corrective: true,
        manual_correction_speed: 5.0,
        wipe_ground_closing: 7.1,
    };

    /// Skate 3's `physics_mode/hardcore`.
    pub const HARDCORE: ModeTuning = ModeTuning {
        push_dv_start: 0.5,
        push_dv_end: 0.25,
        pump_factor: 18.0,
        pump_max_accel: 8.0,
        pump_unintentional: 0.0,
        jump_low: 1.21,
        jump_high: 1.71,
        grind_lock_dist: 0.15,
        max_auto_spin: 0.5,
        wobble_onset: 12.0,
        wobble_amplitude: 1.0,
        manual_corrective: true,
        manual_correction_speed: 2.0,
        wipe_ground_closing: 7.1,
    };

    /// Per [`Difficulty`].
    pub const BUILT_IN: [ModeTuning; 3] =
        [ModeTuning::EASY, ModeTuning::NORMAL, ModeTuning::HARDCORE];

    /// Takes over one field of the collection (a [`FIELDS`] name). A flag is on when non-zero.
    pub(super) fn set(&mut self, field: &str, v: &[f32]) -> bool {
        let Some(&f) = v.first().filter(|f| f.is_finite()) else {
            return false;
        };
        let slot = match field {
            "MaxPushDVStart" => &mut self.push_dv_start,
            "MaxPushDVEnd" => &mut self.push_dv_end,
            "PumpEffectFactor" => &mut self.pump_factor,
            "Hash_D77AFD320B6241C5" => &mut self.pump_max_accel,
            "UnintentionalPumpScalar" => &mut self.pump_unintentional,
            "JumpMinHeight" => &mut self.jump_low,
            "JumpMaxHeight" => &mut self.jump_high,
            "GrindLockDist" => &mut self.grind_lock_dist,
            "MaxAutoBodySpinSpeed" => &mut self.max_auto_spin,
            "Hash_77AFCE78FE1206CA" => &mut self.wobble_onset,
            "Hash_5B57F2CCCCEEF430" => &mut self.wobble_amplitude,
            "Hash_84CB2F459F3FC811" => &mut self.manual_correction_speed,
            "Wipeout_GroundXZAcceleration" => &mut self.wipe_ground_closing,
            "Hash_F8CBC0F5FEF2240E" => {
                self.manual_corrective = f != 0.0;
                return true;
            }
            _ => return false,
        };
        *slot = f;
        true
    }

    /// The values the riding tuning `t` holds now.
    pub fn of(t: &super::Tuning) -> ModeTuning {
        ModeTuning {
            push_dv_start: t.push_tick_limit_slow,
            push_dv_end: t.push_tick_limit_fast,
            pump_factor: t.pump_factor,
            pump_max_accel: t.pump_max_accel,
            pump_unintentional: t.pump_unintentional,
            jump_low: t.jump_low,
            jump_high: t.jump_high,
            grind_lock_dist: t.grind.lock_dist,
            max_auto_spin: t.max_auto_spin,
            wobble_onset: t.wobble.activation,
            wobble_amplitude: t.wobble.amplitude_scale,
            manual_corrective: t.manual_corrective,
            manual_correction_speed: t.manual_correction_speed,
            wipe_ground_closing: t.wipe_ground_closing,
        }
    }

    /// Puts these values into the riding tuning `t`.
    pub(super) fn write(&self, t: &mut super::Tuning) {
        t.push_tick_limit_slow = self.push_dv_start;
        t.push_tick_limit_fast = self.push_dv_end;
        t.pump_factor = self.pump_factor;
        t.pump_max_accel = self.pump_max_accel;
        t.pump_unintentional = self.pump_unintentional;
        t.jump_low = self.jump_low;
        t.jump_high = self.jump_high;
        t.grind.lock_dist = self.grind_lock_dist;
        t.max_auto_spin = self.max_auto_spin;
        t.wobble.activation = self.wobble_onset;
        t.wobble.amplitude_scale = self.wobble_amplitude;
        t.manual_corrective = self.manual_corrective;
        t.manual_correction_speed = self.manual_correction_speed;
        t.wipe_ground_closing = self.wipe_ground_closing;
    }
}

/// Skate 3's truck tightness (its board setup's "Truck tightness", SK8 `skate-game/src/customiser.rs`):
/// 0 loose .. 1 tight in steps of 0.1, 0.7 by default. Kept in tenths.
pub const TRUCK_TIGHTNESS_STEPS: u8 = 10;
pub const DEFAULT_TRUCK_TIGHTNESS: u8 = 7;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::{Tuning, encode};
    use alloc::vec;

    #[test]
    fn the_row_steps_and_wraps_like_skate_3s() {
        assert_eq!(Difficulty::default(), Difficulty::Normal);
        assert_eq!(Difficulty::Normal.step(1), Difficulty::Hardcore);
        assert_eq!(Difficulty::Hardcore.step(1), Difficulty::Easy);
        assert_eq!(Difficulty::Easy.step(-1), Difficulty::Hardcore);
        for (i, d) in Difficulty::ALL.into_iter().enumerate() {
            assert_eq!(d.index(), i);
            assert_eq!(Difficulty::parse(d.key()), Some(d));
        }
        assert_eq!(Difficulty::parse("motorized"), None);
    }

    #[test]
    fn the_paths_name_every_field_of_every_mode() {
        let mut m = ModeTuning::EASY;
        for (d, paths) in Difficulty::ALL.into_iter().zip(PATHS) {
            for (p, f) in paths.iter().zip(FIELDS) {
                assert_eq!(*p, alloc::format!("physics_mode/{}/{f}", d.key()));
                assert!(m.set(f, &[1.0]));
            }
        }
    }

    #[test]
    fn the_built_in_riding_tuning_is_skate_3s_normal() {
        let t = Tuning::BUILT_IN;
        assert_eq!(t.difficulty, Difficulty::Normal);
        assert_eq!(ModeTuning::of(&t), ModeTuning::NORMAL);
        assert_eq!(t.modes, ModeTuning::BUILT_IN);
        assert_eq!(t.truck_tightness, 0.7);
    }

    #[test]
    fn each_difficulty_selects_its_own_collection() {
        let mut t = Tuning::BUILT_IN;
        for d in Difficulty::ALL {
            t.set_difficulty(d);
            assert_eq!(t.difficulty, d);
            assert_eq!(ModeTuning::of(&t), ModeTuning::BUILT_IN[d.index()]);
        }
        // Easy has no manual corrective force (Normal and Hardcore do).
        t.set_difficulty(Difficulty::Easy);
        assert!(!t.manual_corrective);
        assert_eq!(t.manual_correction_speed, 4.0);
        t.set_difficulty(Difficulty::Hardcore);
        assert!(t.manual_corrective);
        assert_eq!(t.grind.lock_dist, 0.15);
        // Everything else stays.
        let mut normal = Tuning::BUILT_IN;
        normal.set_difficulty(Difficulty::Hardcore);
        normal.set_difficulty(Difficulty::Normal);
        assert_eq!(normal, Tuning::BUILT_IN);
    }

    #[test]
    fn a_rules_pack_fills_every_mode_and_the_selected_one_rides() {
        let data = encode(&[
            ("physics_mode/easy/MaxPushDVStart", vec![2.0]),
            ("physics_mode/normal/MaxPushDVStart", vec![0.7]),
            ("physics_mode/hardcore/Hash_F8CBC0F5FEF2240E", vec![0.0]),
            ("physics_mode/easy/Hash_F8CBC0F5FEF2240E", vec![1.0]),
            ("physics_mode/test/MaxPushDVStart", vec![9.0]),
        ]);
        let mut t = Tuning::BUILT_IN;
        assert_eq!(
            t.apply(&data),
            4,
            "the unused test mode is not one of these"
        );
        assert_eq!(t.push_tick_limit_slow, 0.7, "Normal rides");
        t.set_difficulty(Difficulty::Easy);
        assert_eq!(t.push_tick_limit_slow, 2.0);
        assert!(t.manual_corrective);
        t.set_difficulty(Difficulty::Hardcore);
        assert!(!t.manual_corrective);
        // Values arriving after the choice still reach the riding tuning.
        let mut t = Tuning::BUILT_IN;
        t.set_difficulty(Difficulty::Easy);
        t.apply(&data);
        assert_eq!(t.push_tick_limit_slow, 2.0);
    }
}
