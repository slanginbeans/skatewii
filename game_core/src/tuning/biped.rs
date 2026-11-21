//! Tuning for walking off the board: Skate 3's `physics_biped` (speed against the stick, the sprint,
//! turning, slopes, the jump) and `physics_state_offboard` (how the stick's angle to the facing becomes
//! a move and a turn), as SK8-ENGINE/skate-3-rust-engine reads them (GPL-3.0; skate-core
//! player/offboard: ground_input.rs, movement_intent.rs, movement_velocity.rs, air_launch/jump.rs).
//! The built-in values are Skate 3's; the rules pack (`fg_import rules`) supplies the disc's.

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BipedTuning {
    /// Speed (m/s) against the move amount (0..1): `SpeedVsInput`.
    pub speed_vs_input: Curve,
    /// The sprint's speed against the move amount, and its share against the time sprinting (s): a
    /// burst that fades while the button stays held.
    pub sprint_speed: Curve,
    pub sprint_blend: Curve,
    /// The longest the sprint's time counts (s; the blend curve's last x).
    pub sprint_time_cap: f32,
    /// The target speed's scale against the slope along the facing (degrees, + uphill).
    pub slope_speed: Curve,
    /// The fastest turn (deg/s) against the speed, and how far the turn may change in a tick (deg/s).
    pub turn_vs_speed: Curve,
    pub turn_delta_vs_speed: Curve,
    /// The jump: its height (m) and the share of the ground speed it keeps.
    pub jump_height: f32,
    pub jump_speed_scalar: f32,
    /// The move amount against the stick's angle away from straight back (0..1 of pi), and the turn
    /// (`physics_state_offboard`).
    pub move_vs_stick_angle: Curve,
    pub turn_vs_stick_angle: Curve,
}

impl BipedTuning {
    pub const BUILT_IN: BipedTuning = BipedTuning {
        speed_vs_input: Curve::new(&[
            (0.0, 0.0),
            (0.03647416, 0.0),
            (0.03908795, 1.487603),
            (0.06188925, 1.983471),
            (0.1498371, 2.157025),
            (0.2801303, 2.206612),
            (0.4364821, 2.355372),
            (0.52443, 2.454545),
            (0.6498371, 2.628099),
            (0.747557, 2.603306),
            (0.8045602, 2.628099),
            (0.8973941, 2.628099),
            (0.9381108, 2.677686),
            (0.9429967, 3.396694),
            (0.9429967, 6.0),
            (1.0, 6.0),
        ]),
        sprint_speed: Curve::new(&[
            (0.0, 0.0),
            (0.03647416, 0.0),
            (0.03647416, 10.0),
            (1.0, 10.0),
        ]),
        sprint_blend: Curve::new(&[
            (0.0, 1.0),
            (0.3452769, 1.0),
            (0.7491857, 0.8857143),
            (1.452769, 0.4964286),
            (2.983713, 0.2857143),
            (3.394137, 0.1071429),
            (3.843648, 0.0),
            (4.0, 0.0),
        ]),
        sprint_time_cap: 4.0,
        slope_speed: Curve::new(&[
            (-45.0, 1.857143),
            (-40.45603, 1.728571),
            (-23.59935, 1.342857),
            (-4.983713, 1.0),
            (6.009774, 1.0),
            (35.17915, 0.5785714),
            (43.97394, 0.3428572),
            (45.0, 0.0),
        ]),
        turn_vs_speed: Curve::new(&[
            (0.0, 500.0),
            (1.973941, 500.0),
            (3.127036, 462.5),
            (4.651466, 369.6429),
            (6.234528, 287.5),
            (8.247557, 235.7143),
            (10.7101, 205.3571),
            (12.0, 203.5714),
        ]),
        turn_delta_vs_speed: Curve::new(&[
            (0.0, 90.0),
            (0.5667753, 90.0),
            (2.013029, 61.71429),
            (3.967427, 44.67858),
            (5.765472, 38.57142),
            (7.71987, 26.67858),
            (10.10423, 17.03571),
            (12.0, 9.321427),
        ]),
        jump_height: 0.9,
        jump_speed_scalar: 0.8,
        move_vs_stick_angle: Curve::new(&[
            (0.0, 0.0),
            (0.05374593, 0.0),
            (0.3387622, 0.08928572),
            (0.4511401, 0.2535714),
            (0.5032573, 0.5071428),
            (0.5472313, 0.8928571),
            (0.6156352, 1.0),
            (0.9983714, 1.0),
        ]),
        turn_vs_stick_angle: Curve::new(&[
            (0.0, 1.7),
            (0.1596091, 1.36),
            (0.5635179, 1.159643),
            (0.6921824, 0.8628572),
            (0.757329, 0.6114286),
            (0.9185668, 0.08571429),
            (0.97557, 0.0),
            (1.0, 0.0),
        ]),
    };

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
            "physics_biped/default/SpeedVsInput" => curve!(self.speed_vs_input),
            "physics_biped/default/Hash_7209DCFDF3015EBF" => curve!(self.sprint_speed),
            "physics_biped/default/Hash_6B93C51256A30FB4" => match c {
                Some(c) => {
                    self.sprint_blend = c;
                    self.sprint_time_cap = c.last_x();
                    true
                }
                None => false,
            },
            "physics_biped/default/Hash_31309236050A8F09" => curve!(self.slope_speed),
            "physics_biped/default/TurnVsSpeed" => curve!(self.turn_vs_speed),
            "physics_biped/default/TurnDeltaVsSpeed" => curve!(self.turn_delta_vs_speed),
            "physics_biped/default/JumpHeight" => scalar!(self.jump_height),
            "physics_biped/default/JumpSpeedScalar" => scalar!(self.jump_speed_scalar),
            "physics_state_offboard/default/Hash_DF759B46440F16E9" => {
                curve!(self.move_vs_stick_angle)
            }
            "physics_state_offboard/default/TurnVsStickAngle" => curve!(self.turn_vs_stick_angle),
            _ => false,
        }
    }
}

pub const PATHS: &[&str] = &[
    "physics_biped/default/SpeedVsInput",
    "physics_biped/default/Hash_7209DCFDF3015EBF",
    "physics_biped/default/Hash_6B93C51256A30FB4",
    "physics_biped/default/Hash_31309236050A8F09",
    "physics_biped/default/TurnVsSpeed",
    "physics_biped/default/TurnDeltaVsSpeed",
    "physics_biped/default/JumpHeight",
    "physics_biped/default/JumpSpeedScalar",
    "physics_state_offboard/default/Hash_DF759B46440F16E9",
    "physics_state_offboard/default/TurnVsStickAngle",
];
