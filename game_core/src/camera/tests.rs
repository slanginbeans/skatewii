//! The camera against synthetic parks and scripted skaters (flatground's own shots and shot choice).

use alloc::vec::Vec;

use super::*;
use crate::math::{PI, v3, yaw_dir};
use crate::math::{sinf, wrap_angle};
use crate::park::{Surface, Terrain};
use crate::skater::{AirContact, Flight, Manual, Plan};
use rig::{Placement, Positioner};
use shot::ShotDef;

const DT: f32 = 1.0 / 60.0;

/// Flat ground everywhere, or with a block or a drop.
struct Ground {
    /// Height of x, z.
    height: fn(f32, f32) -> f32,
}

impl Terrain for Ground {
    fn ground(&self, x: f32, z: f32, _max: f32) -> Surface {
        Surface {
            height: (self.height)(x, z),
            normal: Vec3::UP,
        }
    }
    fn occluded(&self, p: Vec3, q: Vec3) -> bool {
        self.raycast(p, (q - p).normalize_or(Vec3::UP), (q - p).length())
            .is_some()
    }
    fn spawn(&self) -> (Vec3, f32) {
        (Vec3::ZERO, 0.0)
    }
}

const FLAT: Ground = Ground { height: |_, _| 0.0 };

fn setup(park: &dyn Terrain) -> (Skater, CameraMan) {
    let s = Skater::spawn(park);
    let c = CameraMan::new(&s, park, CamData::default(), CamTuning::BUILT_IN);
    (s, c)
}

/// Rolls the skater along yaw `yaw(t)` at `speed` for `ticks`.
fn ride(
    s: &mut Skater,
    c: &mut CameraMan,
    park: &dyn Terrain,
    ticks: usize,
    speed: f32,
    mut yaw: impl FnMut(f32) -> f32,
) {
    for i in 0..ticks {
        let y = yaw(i as f32 * DT);
        s.set_heading(y);
        s.speed = speed;
        s.vel = yaw_dir(y) * speed;
        s.pos += s.vel * DT;
        c.update(s, park, DT);
    }
}

fn deg(r: f32) -> f32 {
    r / RAD
}

#[test]
fn the_slow_shot_holds_at_push_speed_and_the_fast_one_comes_from_speed() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 300, 8.0, |_| 0.0);
    assert_eq!(c.shot_name, "bl_chase");
    // The blend's value creeps up to the speed but stays under its first point: all slow.
    let v = c.shots.blend_value(0).unwrap();
    assert!(v <= 9.0, "{v}");
    assert!((c.shown().distance - 1.9).abs() < 0.02, "{:?}", c.shown());
    // Bombing a hill: the fast shot.
    ride(&mut s, &mut c, &FLAT, 900, 25.0, |_| 0.0);
    assert!(
        (c.shown().distance - 2.4).abs() < 0.05,
        "{}",
        c.shown().distance
    );
}

#[test]
fn steering_swings_the_framing_yaw_and_rolls() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 120, 5.0, |_| 0.0);
    let straight = c.shown().framing;
    let roll0 = c.orient.roll;
    s.steer = 1.0;
    ride(&mut s, &mut c, &FLAT, 240, 5.0, |_| 0.0);
    let left = c.shown().framing;
    assert!(
        deg(left[1]) > deg(straight[1]) + 4.0,
        "{left:?} {straight:?}"
    );
    assert!(deg(c.orient.roll) > deg(roll0) + 1.0, "{}", c.orient.roll);
    s.steer = -1.0;
    ride(&mut s, &mut c, &FLAT, 300, 5.0, |_| 0.0);
    let right = c.shown().framing;
    assert!(deg(right[1]) < -10.0, "{right:?}");
    assert!(c.orient.roll < 0.0);
}

#[test]
fn the_heading_ignores_wiggles_and_follows_a_big_turn() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 120, 6.0, |_| 0.0);
    let start = c.rig.heading.pos();
    let mut most = 0.0f32;
    for k in 0..120 {
        ride(&mut s, &mut c, &FLAT, 1, 6.0, |_| {
            3.0 * RAD * sinf(k as f32 * DT * 2.0 * PI * 1.5)
        });
        most = most.max(wrap_angle(c.rig.heading.pos() - start).abs());
    }
    assert!(deg(most) < 1.5, "wiggles moved the camera {}", deg(most));
    // A quarter turn, held: the camera comes round.
    ride(&mut s, &mut c, &FLAT, 150, 6.0, |_| PI / 2.0);
    let err = wrap_angle(c.rig.heading_target - c.rig.heading.pos());
    assert!(deg(err).abs() < 12.0, "{}", deg(err));
    // The camera ends up behind the skater (on the other side from the travel).
    let back = (c.camera().eye - s.pos)
        .horizontal()
        .normalize_or(Vec3::ZERO);
    assert!(back.dot(yaw_dir(PI / 2.0)) < -0.8, "{back:?}");
}

/// A planned flight from where the skater is: up at `vy`, along +z at `vz`.
fn plan(s: &Skater, vy: f32, vz: f32) -> Plan {
    let g = 9.8;
    let t = 2.0 * vy / g;
    let flight = Flight {
        pos: s.pos,
        vel: v3(0.0, vy, vz),
        gravity: g,
    };
    Plan {
        flight,
        start: 0.0,
        adopted: 0.0,
        offset: Vec3::ZERO,
        contact: AirContact {
            time: t,
            point: flight.at(t),
            normal: Vec3::UP,
            surface: 0,
        },
        normal: Vec3::UP,
        heading: None,
        index: 0,
        pass: 0,
        grind: None,
    }
}

/// Flies the plan; returns the shots chosen and the lowest and highest eye heights.
fn fly(s: &mut Skater, c: &mut CameraMan, vy: f32) -> (Vec<String>, f32, f32) {
    let p = plan(s, vy, 6.0);
    let duration = p.contact.time;
    s.plan = Some(p);
    let mut shots = Vec::new();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut t = 0.0;
    while t < duration {
        t += DT;
        s.force_state(crate::skater::State::PhysicsAir);
        s.air_time = t;
        s.states.time = t;
        s.pos = p.flight.at(t);
        s.vel = p.flight.vel_at(t);
        c.update(s, &FLAT, DT);
        if !shots.contains(&c.shot_name) {
            shots.push(c.shot_name.clone());
        }
        let y = c.camera().eye.y;
        lo = lo.min(y);
        hi = hi.max(y);
    }
    s.force_state(crate::skater::State::PhysicsGround);
    s.plan = None;
    s.pos.y = 0.0;
    (shots, lo, hi)
}

#[test]
fn a_small_ollie_keeps_its_height_and_a_big_air_pulls_back() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 180, 6.0, |_| 0.0);
    let ground = c.camera().eye.y;
    // An ollie 0.35 m high.
    let (shots, _, hi) = fly(&mut s, &mut c, 2.6);
    assert!(shots.iter().any(|n| n == "low_ollie"), "{shots:?}");
    // The board rises 0.35 m; the camera much less (the take-off tick still follows the board, and the
    // anchor change's collision check nudges it for a moment, as Skate 3's does).
    assert!(
        hi - ground < 0.16,
        "the camera chased the ollie: {ground} -> {hi}"
    );
    ride(&mut s, &mut c, &FLAT, 180, 6.0, |_| 0.0);
    // Nearly two seconds in the air.
    let (shots, _, hi) = fly(&mut s, &mut c, 9.0);
    assert!(shots.iter().any(|n| n == "bl_air"), "{shots:?}");
    assert!(
        hi - ground > 1.0,
        "a big air is followed up: {ground} -> {hi}"
    );
}

#[test]
fn a_manual_has_its_own_shot() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 60, 4.0, |_| 0.0);
    s.manual = Some(Manual::default());
    ride(&mut s, &mut c, &FLAT, 120, 4.0, |_| 0.0);
    assert_eq!(c.shot_name, "bl_manual");
    // Close and low at manual speed.
    assert!(c.shown().distance < 1.6, "{}", c.shown().distance);
    s.manual = None;
    ride(&mut s, &mut c, &FLAT, 10, 4.0, |_| 0.0);
    assert_eq!(c.shot_name, "bl_chase");
}

#[test]
fn ledge_grinds_shoot_from_the_open_side() {
    // Travelling +z, a ledge whose top is on +x (the left).
    assert_eq!(
        subject::ledge_side(v3(0.0, 0.0, 1.0), v3(1.0, 0.0, 0.0)),
        (true, false)
    );
    assert_eq!(
        subject::ledge_side(v3(0.0, 0.0, 1.0), v3(-1.0, 0.0, 0.0)),
        (false, true)
    );
    let set = ShotSet::built_in();
    let mut cond = Conditions {
        grinding: true,
        ledge_left: true,
        ..Default::default()
    };
    assert_eq!(select::built_in(&cond, &set).name, "bl_grind_ledge_left");
    cond.ledge_left = false;
    cond.ledge_right = true;
    assert_eq!(select::built_in(&cond, &set).name, "bl_grind_ledge_right");
    cond.ledge_right = false;
    assert_eq!(select::built_in(&cond, &set).name, "bl_grind");
    // The ledge shots sit on opposite sides.
    let side = |n: &str| set.defs[set.find(n).unwrap()].shot;
    let l = side("ledge_left_slow").heading;
    let r = side("ledge_right_slow").heading;
    assert!(l * r < 0.0);
}

#[test]
fn riding_fakie_flips_the_mirror() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 120, 5.0, |_| 0.0);
    assert!(c.state.heading_mirror.pos > 0.99);
    let arm = c.shown().arm_heading;
    // Rolling tail first towards +z: the nose points -z.
    s.fakie = true;
    for _ in 0..240 {
        s.set_heading(PI);
        s.speed = -5.0;
        s.vel = v3(0.0, 0.0, 5.0);
        s.pos += s.vel * DT;
        c.update(&s, &FLAT, DT);
    }
    assert!(
        c.state.heading_mirror.pos < -0.99,
        "{:?}",
        c.state.heading_mirror
    );
    assert!(
        c.state.framing_mirror.pos < -0.9,
        "{:?}",
        c.state.framing_mirror
    );
    // The same travel, the shot's heading on the other side of straight behind.
    let a = wrap_angle(arm - PI);
    let b = wrap_angle(c.shown().arm_heading - PI);
    assert!(a * b < 0.0, "{} {}", deg(a), deg(b));
}

#[test]
fn shot_changes_ease_over_the_transition_time() {
    let mut set = ShotSet::built_in();
    let a = set.find("chase_slow").unwrap();
    let mut def: ShotDef = set.defs[a];
    def.key = shot::name_hash("test_far");
    def.shot.distance = 5.0;
    def.transition_time = 1.0;
    set.defs.push(def);
    let b = set.defs.len() - 1;
    let inputs = BlendInputs {
        mirror: 1.0,
        ..Default::default()
    };
    let mut m = ShotManager::new();
    m.choose(&set, a, false, &inputs);
    m.update(&set, DT, &inputs);
    m.instant = false;
    let from = m.interpolated.distance;
    assert!(m.choose(&set, b, false, &inputs));
    let mut at = |secs: f32| {
        for _ in 0..(secs / DT).round() as usize {
            m.update(&set, DT, &inputs);
        }
        (m.interpolated.distance - from) / (5.0 - from)
    };
    let quarter = at(0.25);
    let half = at(0.25);
    let done = at(0.6);
    // Sine-eased: slow at first, half way at half time, there by the end.
    assert!(quarter < 0.2, "{quarter}");
    assert!((half - 0.5).abs() < 0.05, "{half}");
    assert!((done - 1.0).abs() < 1e-4, "{done}");
    // Choosing the same shot again doesn't restart it.
    assert!(!m.choose(&set, b, false, &inputs));
}

fn placement(anchor: Vec3) -> Placement {
    Placement {
        anchor,
        heading: PI,
        elevation: 7.0 * RAD,
        reference_heading: PI,
        reference_elevation: 7.0 * RAD,
        distance: 2.0,
        min_distance: 0.5,
        collision: true,
        bypass: false,
        starts: [
            anchor + v3(0.0, 0.65, 0.0),
            anchor + v3(0.0, 1.3, 0.0),
            anchor + v3(0.0, 0.1, 0.0),
        ],
    }
}

#[test]
fn collision_pulls_in_only_when_all_three_lines_are_blocked() {
    let t = CamTuning::BUILT_IN;
    let anchor = v3(0.0, 0.3, 0.0);
    // A low kerb behind the skater: blocks the line from the feet only.
    let kerb = Ground {
        height: |_, z| if (-1.2..-1.0).contains(&z) { 0.5 } else { 0.0 },
    };
    let mut p = Positioner {
        distance: tracker::Scalar::at(2.0),
        ..Default::default()
    };
    p.update(&kerb, &placement(anchor), &t, DT);
    assert!(!p.hit);
    assert!((p.distance.pos - 2.0).abs() < 0.05);
    // A wall behind: all three blocked, the camera comes in in front of it at once.
    let wall = Ground {
        height: |_, z| if (-1.2..-1.0).contains(&z) { 5.0 } else { 0.0 },
    };
    p.update(&wall, &placement(anchor), &t, DT);
    assert!(p.hit);
    assert!(p.position.z > -1.0 && p.distance.pos < 1.2, "{p:?}");
    assert!(p.available < 1.3, "{}", p.available);
    // The collision raise: the camera climbs while the wall is there.
    let mut state = State::default();
    let mut r = Rig {
        positioner: p,
        ..Default::default()
    };
    let (s, _) = setup(&FLAT);
    let mut h = SubjectHistory::default();
    let subj = h.read(&s, &FLAT, false, Vec3::ZERO);
    let shot = ShotSet::built_in().defs[0].shot;
    for _ in 0..30 {
        bind_shot(
            &mut state,
            &mut r,
            &shot,
            &subj,
            &DropPredictor::default(),
            0.0,
            true,
            false,
            true,
            false,
        );
    }
    assert!(
        deg(r.additional_elevation) > 10.0,
        "{}",
        deg(r.additional_elevation)
    );
}

#[test]
fn the_drop_predictor_raises_before_a_ledge() {
    let t = CamTuning::BUILT_IN;
    let ledge = Ground {
        height: |_, z| if z > 3.0 { -2.0 } else { 0.0 },
    };
    let mut d = DropPredictor::default();
    for _ in 0..60 {
        d.update(&FLAT, v3(0.0, 0.1, 0.0), v3(0.0, 0.0, 5.0), true, false, &t);
    }
    assert_eq!(d.elevation, 0.0, "flat ground");
    for _ in 0..60 {
        d.update(
            &ledge,
            v3(0.0, 0.1, 0.0),
            v3(0.0, 0.0, 5.0),
            true,
            false,
            &t,
        );
    }
    assert!(deg(d.elevation) > 15.0, "{}", deg(d.elevation));
    assert!(d.depth > 1.9);
    // Turned off by the shot: back down.
    for _ in 0..120 {
        d.update(
            &ledge,
            v3(0.0, 0.1, 0.0),
            v3(0.0, 0.0, 5.0),
            false,
            false,
            &t,
        );
    }
    assert_eq!(d.elevation, 0.0);
}

#[test]
fn wiping_out_cuts_to_the_high_shot() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 60, 5.0, |_| 0.0);
    s.force_state(crate::skater::State::WipeoutGround);
    s.states.time = 0.0;
    ride(&mut s, &mut c, &FLAT, 120, 2.0, |_| 0.0);
    assert_eq!(c.shot_name, "wipeout");
    let eye = c.camera().eye;
    assert!(eye.y - s.pos.y > 1.0, "{eye:?}");
}

#[test]
fn every_built_in_choice_has_a_shot() {
    let set = ShotSet::built_in();
    for n in [
        "bl_chase",
        "bl_turning",
        "bl_manual",
        "low_ollie",
        "bl_air",
        "bl_grind",
        "bl_grind_ledge_left",
        "bl_grind_ledge_right",
        "grind_exit",
        "wipeout",
        "offboard_ground",
    ] {
        assert!(set.find(n).is_some(), "{n}");
    }
}

#[test]
fn the_lens_gives_skate_3s_field_of_view() {
    // Wider on a 4:3 TV than on a 16:9 one (the frame's angle over sqrt(aspect + 1)).
    let narrow = lens_fov(12.0, 16.0 / 9.0);
    let square = lens_fov(12.0, 4.0 / 3.0);
    assert!(square > narrow);
    assert!((60.0..80.0).contains(&narrow) && (60.0..80.0).contains(&square));
}

#[test]
fn the_camera_steps_the_same_at_any_frame_rate_of_the_game() {
    // The camera only ever steps in ticks: two cameras fed the same ticks agree exactly.
    let (mut s1, mut c1) = setup(&FLAT);
    let (mut s2, mut c2) = setup(&FLAT);
    ride(&mut s1, &mut c1, &FLAT, 200, 7.0, |t| 0.4 * sinf(t));
    ride(&mut s2, &mut c2, &FLAT, 200, 7.0, |t| 0.4 * sinf(t));
    assert_eq!(c1.camera().eye, c2.camera().eye);
}

#[test]
fn turning_reads_the_stick_when_it_agrees_and_is_larger() {
    use subject::steering_for_turn;
    // The stick hard over before the smoothed turn has caught up: the stick.
    assert_eq!(steering_for_turn(0.4, 1.0, 0.0), 1.0);
    // A kick turn's input when it is the larger.
    assert_eq!(steering_for_turn(0.2, 0.3, 0.9), 0.9);
    // Disagreeing in sign, or smaller: the physics' turn.
    assert_eq!(steering_for_turn(0.5, -1.0, 0.0), 0.5);
    assert_eq!(steering_for_turn(0.8, 0.3, 0.0), 0.8);
}

#[test]
fn the_turning_clock_restarts_on_a_stick_held_over() {
    let (mut s, mut c) = setup(&FLAT);
    ride(&mut s, &mut c, &FLAT, 60, 5.0, |_| 0.0);
    assert!(c.state.centred_time > 0.5);
    // The smoothed turn is still small; the stick is hard over (past the dead zone).
    s.steer = 0.3;
    s.turn_input = 1.0;
    ride(&mut s, &mut c, &FLAT, 1, 5.0, |_| 0.0);
    assert_eq!(c.state.centred_time, 0.0);
}
