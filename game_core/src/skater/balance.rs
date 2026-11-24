//! Grind balance, as Skate 3 holds its rigid board on a rail (after SK8-ENGINE/skate-3-rust-engine,
//! GPL-3.0: `physics/grind_forces{,/slide,/support,/post}.rs`, `grind_contact/balance.rs` and skate-game
//! `physics/grind/contact.rs`, `grind_trick.rs`).
//!
//! Skate 3's board on a rail is a rigid body: the family's frame drives its orientation and forces move
//! it, each family in its own order (SK8 grind/contact.rs). flatground keeps the board's motion along the
//! rail and its pose as before, and adds the one degree of freedom the forces act on across the rail:
//! how far the family's reference point (the deck's middle for a 50-50 and the slides, the truck on the
//! rail for a 5-0, the tip for a blunt) has moved off the rail, and how fast. Every force here is
//! Skate 3's (N), divided by the effective mass [`super::GRIND_MASS`]:
//!
//! - **Pins** (`lateral_pin`, the 50-50's `Align(800, 0, 0.07)`, the 5-0's `Align(3000, 0.234, 0.07)`,
//!   the blunt's `Align(3000, 0.39, 0.025)`): a spring pulling the reference back over the rail, damped
//!   by 0.133 of its strength, weakened as the grind leans off (`PinVsSlope`, against the grind normal's
//!   up), at full strength while the coping holds the board.
//! - **Slides** (`slide::control`): the sticks' x push a boardslide across the rail (25 N a unit, a
//!   darkslide 40 N fading out 0.16 m off the middle); within 0.16 m of the middle the board's sideways
//!   speed is damped (20 N per m/s), past it the board is pulled back (10 N) and stopped if moving out;
//!   on a ledge only the push acts. Winding up a pop pulls it back in (201 N past 0.12 m).
//! - **Tip slides** (`TipStability`): the main stick's x pushes the board across (37 N a unit), or for 8
//!   ticks after dropping in onto the tip faster than 1.8 m/s it is pulled in (39 N; SK8 entry.rs also
//!   counts a tip taken from a 5-0, which flatground's grinds don't change into).
//! - **Exit lean** (`support::exit_lean`): leaning off a ledge pushes the board to the low side, 65 N at
//!   full `ExitAssistVsLeanAngle`.
//! - **Friction** (`friction`) damps the whole velocity in the rail's plane: along the rail and across.
//!
//! The grind is lost when the rail leaves the box it was in (falling off the rail: the truck's width for
//! the grinds, the deck between the trucks for a boardslide, the tip for tip slides; a tip slide pushed
//! in past the truck is a boardslide), or wiped out (SK8 post.rs): the velocity the rail stops when the
//! board comes onto it or hits it (its closing speed), seen in the rider's frame, more than
//! `Wipeout_GrindXZDeck` across the ground plane. A boardslide that stalls on a rail (not a ledge) is a
//! wipeout too (SK8 involuntary.rs: rails request wipeout 10).

use crate::math::Vec3;
use crate::tuning::GrindTuning;

/// Pin strengths (N/m) and where the reference sits along the board (m), SK8 grind/contact.rs.
pub const PIN_FIFTY: f32 = 800.0;
pub const PIN_FIVE: f32 = 3000.0;
pub const PIN_BLUNT: f32 = 3000.0;
pub const BLUNT_REACH: f32 = 0.39;
/// The pins' damping: this share of the strength per m/s (SK8 `0x3e083127`).
pub const PIN_DAMPING: f32 = 0.133;
/// Slides: the push per unit of stick (N), the darkslide's and its fade per metre off the middle.
pub const SLIDE_PUSH: f32 = 25.0;
pub const DARK_PUSH: f32 = 40.0;
pub const DARK_FADE: f32 = 6.25;
/// Slides: the middle's half width (m), the pull back past it (N), the damping inside it (N per m/s).
pub const SLIDE_EDGE: f32 = 0.16;
pub const SLIDE_BACK: f32 = 10.0;
pub const SLIDE_DAMPING: f32 = 20.0;
/// Winding up a pop: past this far off the middle (m) the slide is pulled in this hard (N).
pub const SLIDE_JUMP_EDGE: f32 = 0.12;
pub const SLIDE_JUMP_BACK: f32 = 201.0;
/// Tip slides: the stick's push (N a unit), the pull in after dropping in fast onto the tip (N), for how
/// many ticks, from what speed (m/s; SK8 entry.rs `tipslide_frames`).
pub const TIP_PUSH: f32 = 37.0;
pub const TIP_HOLD: f32 = 39.0;
pub const TIP_HOLD_TICKS: u32 = 8;
pub const TIP_HOLD_SPEED: f32 = 1.8;
/// The exit lean's push at full assist (N, SK8 support.rs).
pub const EXIT_LEAN: f32 = 65.0;
/// Grind grabs: the 5-0's free end is held at least this high (Skate 3's grind-grab states'
/// `PhysGrindGrabMinHeight`), pushed up 10 per unit short of it (SK8 control.rs).
pub const GRAB_MIN_HEIGHT: f32 = 0.2;
pub const GRAB_LIFT: f32 = 10.0;

/// SK8 `pin_slope`: the pins' share of their strength, full while the coping holds the board.
pub fn pin_slope(t: &GrindTuning, normal_up: f32, relief: f32) -> f32 {
    if relief > 0.0 {
        1.0
    } else {
        t.pin_vs_slope.eval(normal_up)
    }
}

/// SK8 `lateral_pin` across the rail: `error` is the rail's offset from the reference (m, along the
/// across axis), `vel` the board's speed along it. No error, no force (Skate 3 skips the call).
pub fn pin(strength: f32, error: f32, vel: f32, slope: f32) -> f32 {
    if error == 0.0 {
        return 0.0;
    }
    (error * strength - strength * PIN_DAMPING * vel) * slope
}

/// SK8 `slide::control` across the rail: `offset` is the deck's middle off the rail (m), `vel` its
/// speed across, `push` the sticks' x. Returns the force (N) and whether the board's sideways speed is
/// stopped first (it was moving out past the middle).
pub fn slide(
    dark: bool,
    offset: f32,
    vel: f32,
    push: f32,
    ledge: bool,
    preparing_jump: bool,
) -> (f32, bool) {
    let inward = if offset > 0.0 { -1.0 } else { 1.0 };
    if preparing_jump {
        let f = if offset.abs() > SLIDE_JUMP_EDGE {
            inward * SLIDE_JUMP_BACK
        } else {
            0.0
        };
        return (f, false);
    }
    let strength = if dark {
        (1.0 - offset.abs() * DARK_FADE) * DARK_PUSH
    } else {
        SLIDE_PUSH
    };
    let translate = push * strength;
    if ledge {
        return (translate, false);
    }
    if offset.abs() > SLIDE_EDGE {
        (inward * SLIDE_BACK, inward * vel < 0.0)
    } else {
        (translate - SLIDE_DAMPING * vel, false)
    }
}

/// SK8 `TipStability` across the rail (travel-directed across): the stick's push, or the pull in
/// (`inward`: the sign towards the rail) just after riding fast onto the tip.
pub fn tip(balance: f32, hold: bool, inward: f32) -> f32 {
    if hold {
        inward * TIP_HOLD
    } else {
        balance * TIP_PUSH
    }
}

/// SK8 `support::exit_lean`: the push towards the low side (N) at this lean (degrees).
pub fn exit_lean(t: &GrindTuning, angle: f32) -> f32 {
    if angle > 0.0 {
        t.exit_assist.eval(angle) * EXIT_LEAN
    } else {
        0.0
    }
}

/// The closing speed the rail stopped (`removed`), as Skate 3's grind wipeout reads it (SK8 post.rs):
/// in the rider's frame (`up`, `fwd`), its length across the ground plane (x and z).
pub fn closing_xz(removed: Vec3, up: Vec3, fwd: Vec3) -> f32 {
    let right = up.cross(fwd);
    let (x, z) = (removed.dot(right), removed.dot(fwd));
    libm::sqrtf(x * x + z * z)
}

/// SK8 control.rs for a grind grab in a 5-0: the free end (`height`, rad, + up) held at least
/// `min_height` by the stick's pitch (`lift`: the pitch's sign that lifts it). `None`: no change.
pub fn grab_lift(min_height: f32, height: f32, lift: f32) -> Option<f32> {
    (min_height > 0.0 && height <= min_height).then(|| {
        let v = (min_height - height) * lift * GRAB_LIFT;
        if lift > 0.0 { v.min(lift) } else { v.max(lift) }
    })
}

/// Skate 3's grind grabs (its motion graph's `<grind>_<grab>` states under `OnBoard/Grinding`): for a
/// grind (the graph name's base, without side or `BF_`), the grab the frontside hand plays and the grab
/// the backside hand plays.
pub fn grind_grabs(base: &str) -> [Option<&'static str>; 2] {
    match base {
        "50_50" => [Some("FSGRAB"), Some("BSGRAB")],
        "5_O" | "SALAD" => [Some("FSGRAB"), Some("NOSEGRAB")],
        "NOSEGRIND" | "CROOK" | "OVERCROOK" => [Some("TAILGRAB"), Some("BSGRAB")],
        "NOSESLIDE" | "NOSEBLUNT" => [Some("TAILGRAB"), None],
        "BLUNT" => [None, Some("TAILGRAB")],
        "TAILSLIDE" => [None, Some("NOSEGRAB")],
        _ => [None, None],
    }
}

/// How a grind grab is shown after the grind: `FS GRAB`, `NOSEGRAB` ...
pub fn grab_display(grab: &str) -> &'static str {
    match grab {
        "FSGRAB" => "FS GRAB",
        "BSGRAB" => "BS GRAB",
        "NOSEGRAB" => "NOSEGRAB",
        "TAILGRAB" => "TAILGRAB",
        _ => "GRAB",
    }
}

/// The grab a scored grind-grab name ends with (`FS_5_O_FSGRAB`: `FSGRAB`).
pub fn grab_of(scoring: &str) -> Option<&'static str> {
    ["FSGRAB", "BSGRAB", "NOSEGRAB", "TAILGRAB"]
        .into_iter()
        .find(|g| scoring.len() > g.len() && scoring.ends_with(g))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;
    use crate::tuning::Tuning;

    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn pins_pull_back_over_the_rail_damped_and_weaker_leaning_off() {
        let t = Tuning::BUILT_IN.grind;
        // SK8's leaf test: 100 N/m, 1 m off, moving 2 m/s along the error, half strength.
        near(
            pin(100.0, 1.0, 2.0, 0.5),
            (100.0 - 100.0 * 0.133 * 2.0) * 0.5,
        );
        assert_eq!(pin(800.0, 0.0, 3.0, 1.0), 0.0, "no error, no force");
        // A board 5 cm off, at rest, is pulled back towards the rail.
        assert!(pin(PIN_FIFTY, -0.05, 0.0, 1.0) < 0.0);
        // PinVsSlope: full upright, nothing leaning 45 degrees off, full on coping.
        near(pin_slope(&t, 1.0, 0.0), 1.0);
        near(pin_slope(&t, libm::cosf(45f32.to_radians()), 0.0), 0.0);
        assert!(pin_slope(&t, libm::cosf(15f32.to_radians()), 0.0) < 0.8);
        near(pin_slope(&t, 0.5, 0.2), 1.0);
    }

    #[test]
    fn slides_push_with_the_stick_and_hold_inside_the_middle() {
        // SK8's leaf test: 0.5 stick, at the middle, moving 2 m/s across.
        let (f, stop) = slide(false, 0.0, 2.0, 0.5, false, false);
        near(f, 12.5 - 40.0);
        assert!(!stop);
        // A darkslide's push fades: half of it 8 cm off.
        near(slide(true, 0.08, 0.0, 0.5, false, false).0, 10.0);
        // Past 0.16 m: pulled back, and stopped if still moving out.
        let (f, stop) = slide(false, 0.2, 1.0, 1.0, false, false);
        near(f, -10.0);
        assert!(stop);
        assert!(!slide(false, 0.2, -1.0, 1.0, false, false).1);
        // On a ledge only the push.
        near(slide(false, 0.3, 5.0, 1.0, true, false).0, 25.0);
        // Winding up: pulled in past 0.12 m.
        near(slide(false, -0.13, 0.0, 1.0, false, true).0, 201.0);
        assert_eq!(slide(false, 0.12, 0.0, 1.0, false, true).0, 0.0);
    }

    #[test]
    fn tip_slides_follow_the_stick_and_lean_offs_push_to_the_low_side() {
        let t = Tuning::BUILT_IN.grind;
        near(tip(-1.0, false, 1.0), -37.0);
        near(tip(-1.0, true, 1.0), 39.0);
        assert_eq!(exit_lean(&t, 0.0), 0.0);
        assert_eq!(
            exit_lean(&t, 10.0),
            0.0,
            "ExitAssistVsLeanAngle is 0 to 11 degrees"
        );
        near(exit_lean(&t, 60.0), 65.0);
    }

    #[test]
    fn the_closing_speed_counts_across_the_riders_ground_plane_only() {
        let (up, fwd) = (v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0));
        // Straight down into the rail: none of it across the ground plane.
        near(closing_xz(v3(0.0, -9.0, 0.0), up, fwd), 0.0);
        near(closing_xz(v3(3.0, -9.0, 4.0), up, fwd), 5.0);
        // The rider rolled 45 degrees: half the drop shows sideways.
        let s = core::f32::consts::FRAC_1_SQRT_2;
        let rolled = v3(s, s, 0.0);
        near(closing_xz(v3(0.0, -9.0, 0.0), rolled, fwd), 9.0 * s);
        assert!(closing_xz(v3(0.0, -9.0, 0.0), rolled, fwd) > Tuning::BUILT_IN.grind.wipeout_xz);
        assert!(closing_xz(v3(0.0, -6.0, 0.0), rolled, fwd) < Tuning::BUILT_IN.grind.wipeout_xz);
    }

    #[test]
    fn grind_grabs_are_skate_3s() {
        assert_eq!(grind_grabs("50_50"), [Some("FSGRAB"), Some("BSGRAB")]);
        assert_eq!(grind_grabs("CROOK"), [Some("TAILGRAB"), Some("BSGRAB")]);
        assert_eq!(grind_grabs("BLUNT"), [None, Some("TAILGRAB")]);
        assert_eq!(grind_grabs("BOARD"), [None, None]);
        assert_eq!(grab_of("BS_BF_5_O_NOSEGRAB"), Some("NOSEGRAB"));
        assert_eq!(grab_of("FS_5_O"), None);
        assert_eq!(grab_display("TAILGRAB"), "TAILGRAB");
        // A 5-0 with the free end low is lifted to the grab's height, up to a full stick.
        assert_eq!(grab_lift(0.2, 0.25, 1.0), None);
        near(grab_lift(0.2, 0.15, 1.0).unwrap(), 0.5);
        near(grab_lift(0.2, -0.3, -1.0).unwrap(), -1.0);
        assert_eq!(grab_lift(0.0, -0.3, 1.0), None);
    }
}
