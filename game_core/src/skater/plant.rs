//! Foot plants, as Skate 3 does them (after SK8-ENGINE/skate-3-rust-engine, GPL-3.0):
//!
//! - On the ground, a boneless, a fastplant or a no-comply: the rider plants a foot and the clips carry
//!   the board up with the other (`ANIMSKATEBOARD`: SK8 `skate-game/src/physics/boneless.rs`, physical
//!   state 602), then jump off the planted foot on the clip's `FOOTJUMP` ([`foot_jump`]: four point
//!   graphs of `Hash_CCB95A83C78B4FF9`). Skate 3's action graph asks for them with a ground grab (a
//!   trigger fully in) and a push button, its motion graph's `PushTrick` states play them; a no-comply is
//!   a flick while a push's foot is still coming down.
//! - In the air, the foot plant (on a ledge, the coping, a wall: a wallplant): a one-foot grab (a grab and
//!   a push button) lets the free foot reach for whatever the flight is about to meet ([`Manager`]: SK8
//!   `physics/footplant/update.rs`, `pose.rs`, and `skate-core/src/air/known/footplant.rs`); once the foot
//!   touches the rider swings over it on a curve and is thrown off it again ([`AirPlant`]: SK8
//!   `physics/footplant/ground.rs`, physical state 601).
//!
//! Skate 3 runs these on its physical rider; flatground's rider has no body, so the planted foot is
//! where the leg would put it and the centre of mass is the board's point plus its height over the deck.

use libm::acosf;

use crate::math::{PI, Vec3, v3};
use crate::park::Terrain;
use crate::tuning::plant::PlantTuning;

use super::rotate;

/// Skate 3's fixed step.
const STEP: f32 = 1.0 / 60.0;
/// SK8 boneless.rs: the launch keeps at least this speed along the rider's facing (m/s), and a launch
/// changing the speed by more than this keeps the velocity instead (m/s).
pub const JUMP_MIN_FORWARD: f32 = 1.92;
pub const JUMP_MAX_CHANGE: f32 = 10.0;
/// SK8 known/footplant.rs: the manager runs once the air is this old (s, `0x820641A8`).
pub const MIN_AIR_TIME: f32 = 0.1;
/// SK8 update.rs: the planting leg keeps within this of straight down from the rider (degrees).
pub const LEG_MAX_ANGLE: f32 = 35.0;
/// SK8 update.rs: a contact is taken up when it is between these many seconds away.
pub const CONTACT_WINDOW: (f32, f32) = (0.11, 0.3);
/// SK8 pose.rs: the foot plants when the contact is this close (s).
pub const PERFORM_TIME: f32 = 0.017;
/// SK8 motion_gameplay_conditions.rs: `ShouldPrepareOneFootAirForFootplant` (s, `0x820641A8` and
/// `0x8209975C`).
pub const PREPARE_WINDOW: (f32, f32) = (0.1, 0.5);
/// SK8 ground.rs: the flight after the plant starts this far over the planted foot (m).
pub const RELEASE_LIFT: f32 = 0.2;
pub const RELEASE_BOARD_DROP: f32 = 0.1;
/// How far ahead the free foot's flight is looked along (s; SK8 queries a second, the graphs and the
/// manager only read the first half of it), in chords of this length (s): a bounded number of rays a
/// tick for the Wii.
const SWEEP_TIME: f32 = 0.5;
const SWEEP_CHORD: f32 = 1.0 / 30.0;
/// The leg's foot follows its target at most this far a tick (m, SK8 pose.rs' lock).
const FOOT_FOLLOW: f32 = 0.15;

/// The plants Skate 3's motion graph plays on the ground (its `PushTrick` states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlantKind {
    FsBoneless,
    BsBoneless,
    FsFastplant,
    BsFastplant,
    NoComply,
}

impl PlantKind {
    /// From the graph's scored trick (`ScoringTrick`: `FSBoneless`, `NoComply` ...).
    pub fn from_trick(name: &str) -> Option<PlantKind> {
        Some(match name {
            "FSBoneless" => PlantKind::FsBoneless,
            "BSBoneless" => PlantKind::BsBoneless,
            "FSFastPlant" => PlantKind::FsFastplant,
            "BSFastPlant" => PlantKind::BsFastplant,
            "NoComply" => PlantKind::NoComply,
            _ => return None,
        })
    }

    /// Skate 3's trick name (the motion graph's `ScoringTrick`).
    pub fn trick(self) -> &'static str {
        match self {
            PlantKind::FsBoneless => "FSBoneless",
            PlantKind::BsBoneless => "BSBoneless",
            PlantKind::FsFastplant => "FSFastPlant",
            PlantKind::BsFastplant => "BSFastPlant",
            PlantKind::NoComply => "NoComply",
        }
    }

    /// The landed name.
    pub fn name(self) -> &'static str {
        match self {
            PlantKind::FsBoneless => "FS BONELESS",
            PlantKind::BsBoneless => "BS BONELESS",
            PlantKind::FsFastplant => "FS FASTPLANT",
            PlantKind::BsFastplant => "BS FASTPLANT",
            PlantKind::NoComply => "NO COMPLY",
        }
    }

    /// flatground's points (Skate 3's come from its scoring data).
    pub fn points(self) -> u32 {
        match self {
            PlantKind::NoComply => 150,
            _ => 200,
        }
    }

    /// Skate 3's action graph (`OnGround/Grabbing/LeftGrab|RightGrab`, `Goofy|Regular`): a ground grab
    /// with the left trigger or the right, the right foot's push (A, `RightPush`) or the left's (B), for
    /// the rider's poses mirrored (regular stance) or not. The front hand's grab is frontside; the
    /// back foot's push (the usual one) a boneless, the front foot's a fastplant.
    pub fn from_grab(left_trigger: bool, right_push: bool, mirrored: bool) -> PlantKind {
        let fs = left_trigger != mirrored;
        let boneless = right_push != mirrored;
        match (fs, boneless) {
            (true, true) => PlantKind::FsBoneless,
            (false, true) => PlantKind::BsBoneless,
            (true, false) => PlantKind::FsFastplant,
            (false, false) => PlantKind::BsFastplant,
        }
    }

    /// Without the graphs: seconds from the plant to the board being lifted (`ANIMSKATEBOARD`), and on to
    /// the jump (`FOOTJUMP`), as Skate 3's graphs play them at riding speed (its `PushTrick` states'
    /// clips and transitions, measured from the push press; a no-comply from its flick).
    pub fn timing(self) -> (f32, f32) {
        match self {
            PlantKind::FsBoneless | PlantKind::BsBoneless => (0.225, 0.233),
            PlantKind::FsFastplant | PlantKind::BsFastplant => (0.258, 0.167),
            PlantKind::NoComply => (0.125, 0.2),
        }
    }
}

/// Skate 3's name for a foot plant in the air (its graphs' `ScoringGrabs`): the grab's side (the
/// unmirrored rider's left hand: `fsgrab`) and the free foot's (the right: `fs`).
pub fn footplant_name(fs_grab: bool, fs_foot: bool) -> &'static str {
    match (fs_grab, fs_foot) {
        (true, true) => "fsgrab_fs_footplant",
        (true, false) => "fsgrab_bs_footplant",
        (false, true) => "bsgrab_fs_footplant",
        (false, false) => "bsgrab_bs_footplant",
    }
}

/// A plant on the ground: from the planted foot to the jump off it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundPlant {
    pub kind: PlantKind,
    /// Seconds since the foot went down.
    pub time: f32,
    /// The clips have the board in hand (`ANIMSKATEBOARD`: SK8 physical state 602, `Boneless`).
    pub lifted: bool,
}

/// SK8 boneless.rs `launch` (TU3 `0x82D4CB18`): the jump off the planted foot. `up`: the ground's up;
/// `current`: the velocity now; `prepared`: the velocity the jump was prepared with; `forward`: the
/// rider's facing along the travel. The planted share keeps the speed along the ground (scaled by
/// `jump_speed_scale`) and jumps `jump_up_vs_normal_y` straight up the ground's normal; the rest keeps
/// the velocity and only sets its up speed. At least [`JUMP_MIN_FORWARD`] is kept along the facing.
pub fn foot_jump(t: &PlantTuning, up: Vec3, current: Vec3, prepared: Vec3, forward: Vec3) -> Vec3 {
    let horizontal = prepared - up * prepared.dot(up);
    let speed = horizontal.length();
    let vertical = t.jump_up_vs_normal_y.eval(up.y) * t.jump_up_vs_speed.eval(speed);
    let scaled = t.jump_speed_scale.eval(speed) * speed;
    let a = current + up * (vertical - current.dot(up));
    let b = horizontal.normalize_or(Vec3::ZERO) * scaled + up * vertical;
    let blend = t.jump_blend_vs_normal_y.eval(up.y);
    let mut v = b * blend + a * (1.0 - blend);
    if (v.length() - current.length()).abs() > JUMP_MAX_CHANGE {
        v = current;
    }
    let along = v.dot(forward).max(0.0);
    if along < JUMP_MIN_FORWARD {
        v += forward * (JUMP_MIN_FORWARD - along);
    }
    v
}

/// What the manager reads each tick in the air.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The centre of mass and its velocity, and gravity (m/s^2, down).
    pub com: Vec3,
    pub vel: Vec3,
    pub gravity: f32,
    /// The rider's up, and the predicted landing's normal (up without one).
    pub up: Vec3,
    pub landing_normal: Vec3,
    /// Seconds to the predicted landing (what the manager guesses the contact at before it has one).
    pub landing_time: f32,
    /// A foot is free to plant (one-foot air: SK8's toe outside the deck's box).
    pub candidate: bool,
    /// The clips are reaching for the plant (`FootPlanting`).
    pub planting: bool,
}

/// SK8's FootPlantManager (TU3 `0x82D6FD60` / `0x82D6F5F0` / `0x82D70070`): where the free foot is
/// about to meet something, and when it plants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manager {
    /// The foot's flight meets something this tick, and in how long (s, -1 none).
    pub hit: bool,
    pub contact_time: f32,
    /// Where (the contact lifted by the foot's volume) and the surface's normal there.
    pub contact: Vec3,
    pub normal: Vec3,
    /// A contact in the window was taken up, seconds since.
    pub active: bool,
    pub elapsed: f32,
    /// The clips asked for the plant (`FootPlanting` while active).
    pub requested: bool,
    /// The foot plants now.
    pub perform: bool,
    /// Where the leg reaches (unit, from the centre of mass) and where it is.
    pub launch: Vec3,
    pub leg: Vec3,
}

impl Default for Manager {
    fn default() -> Self {
        Manager {
            hit: false,
            contact_time: -1.0,
            contact: Vec3::ZERO,
            normal: Vec3::UP,
            active: false,
            elapsed: 0.0,
            requested: false,
            perform: false,
            launch: v3(0.0, -1.0, 0.0),
            leg: v3(0.0, -1.0, 0.0),
        }
    }
}

/// `v` turned towards `to` until it is within `max` (rad) of it (SK8 reckoning
/// `clamp_vector_within_max_angle`).
fn within(v: Vec3, to: Vec3, max: f32) -> Vec3 {
    let v = v.normalize_or(to);
    let angle = acosf(v.dot(to).clamp(-1.0, 1.0));
    if angle <= max {
        return v;
    }
    let axis = to.cross(v);
    let len = axis.length();
    if len < 1e-6 {
        return to;
    }
    rotate(to, axis * (1.0 / len), max)
}

/// The first thing a point flying from `from` at `vel` under `gravity` meets within `SWEEP_TIME`: in how
/// long, where, and its normal.
fn sweep(park: &dyn Terrain, from: Vec3, vel: Vec3, gravity: f32) -> Option<(f32, Vec3, Vec3)> {
    let at = |t: f32| from + vel * t + Vec3::UP * (-0.5 * gravity * t * t);
    let mut t0 = 0.0;
    let mut p0 = from;
    while t0 < SWEEP_TIME {
        let t1 = (t0 + SWEEP_CHORD).min(SWEEP_TIME);
        let p1 = at(t1);
        let d = p1 - p0;
        let len = d.length();
        if len > 1e-6
            && let Some(h) = park.raycast(p0, d * (1.0 / len), len)
            && d.dot(h.normal) < 0.0
        {
            let share = (h.t / len).clamp(0.0, 1.0);
            return Some((t0 + (t1 - t0) * share, h.point, h.normal));
        }
        t0 = t1;
        p0 = p1;
    }
    None
}

impl Manager {
    /// One tick of the manager while it runs (SK8 `update_launch`, `consume_and_submit`,
    /// `update_pose`). The query is answered at once here (Skate 3's comes back the next tick).
    pub fn update(&mut self, t: &PlantTuning, park: &dyn Terrain, f: &Frame) {
        // Where the leg reaches: along the flight at the contact, away from the landing, within 35
        // degrees of straight down from the rider.
        let at = if self.hit {
            self.contact_time
        } else {
            f.landing_time.max(0.0)
        };
        let future = f.vel + Vec3::UP * (-f.gravity * at);
        let reach = future.normalize_or(Vec3::ZERO) - f.landing_normal;
        let down = f.up * -1.0;
        self.launch = within(reach, down, LEG_MAX_ANGLE * (PI / 180.0));
        // The free foot's flight, from the leg's full reach.
        let foot = f.com + self.launch * t.leg_length;
        self.hit = false;
        if let Some((time, point, normal)) = sweep(park, foot, f.vel, f.gravity) {
            self.contact_time = time.clamp(0.0, 1.0);
            self.contact = point + f.landing_normal * t.foot_volume_y_offset;
            self.normal = normal;
            self.hit = true;
            if !self.active {
                if f.candidate
                    && self.contact_time > CONTACT_WINDOW.0
                    && self.contact_time < CONTACT_WINDOW.1
                {
                    self.active = true;
                    self.elapsed = 0.0;
                    self.leg = self.launch;
                } else {
                    self.clear();
                }
            }
            if self.active {
                self.requested |= f.planting;
                self.elapsed += STEP;
            }
        }
        // The foot follows the leg's reach (SK8 pose.rs: the IK target, at most 0.15 m a tick).
        let step = FOOT_FOLLOW / t.leg_length.max(0.1);
        self.leg = within(self.leg, self.launch, step).normalize_or(self.launch);
        self.perform = if self.requested {
            let error = acosf(self.launch.dot(self.leg).clamp(-1.0, 1.0));
            self.hit
                && self.contact_time <= PERFORM_TIME
                && error < t.max_leg_angle_error * (PI / 180.0)
        } else {
            false
        };
    }

    fn clear(&mut self) {
        self.hit = false;
        self.contact_time = 0.0;
        self.contact = Vec3::ZERO;
    }

    /// The manager's outputs as the graphs read them: the contact's time (-1 none).
    pub fn published_contact(&self) -> f32 {
        if self.hit { self.contact_time } else { -1.0 }
    }
}

/// The swing over a planted foot (SK8 physical state 601, `FootPlant`): the centre of mass follows a
/// cubic curve round the foot, then flies off it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirPlant {
    /// The curve's points: from the centre of mass at the plant to over the foot.
    pub curve: [Vec3; 4],
    /// The swing's time (s, -1: none, released at once) and seconds into it.
    pub duration: f32,
    pub elapsed: f32,
    /// The velocity it flies off with.
    pub release: Vec3,
    /// The planted foot, and the normal of what it is planted on.
    pub anchor: Vec3,
    pub normal: Vec3,
}

impl AirPlant {
    /// SK8 ground.rs `Footplant::start` (TU3 `0x82D704B0`): the swing from the centre of mass `com`
    /// moving at `vel` round the foot at `contact`. Its rate is the leg's turn rate scaled by
    /// `radial_speed_scale`; it ends `end_angle` from straight up over the foot (no sooner than
    /// `min_duration`, no later than `max_duration`), `end_leg_length` from it, and flies off along the
    /// swing with `release_outward_speed` out along the leg.
    pub fn start(t: &PlantTuning, com: Vec3, vel: Vec3, contact: Vec3, normal: Vec3) -> AirPlant {
        let mut p = AirPlant {
            curve: [com; 4],
            duration: -1.0,
            elapsed: 0.0,
            release: vel,
            anchor: contact,
            normal,
        };
        let offset = com - contact;
        let radial = offset.normalize_or(Vec3::UP);
        let multiplier = t.radial_speed_scale.eval(vel.dot(radial).abs());
        let rotation = offset.cross(vel);
        let angular_speed = rotation.length();
        if angular_speed <= 1.525_878_9e-5 {
            return p;
        }
        let axis = rotation * (1.0 / angular_speed);
        let rate = angular_speed * multiplier;
        // The leg's angle from straight up, seen along the swing's axis.
        let side = axis * -1.0;
        let projected_up = (Vec3::UP - side * Vec3::UP.dot(side)).normalize_or(Vec3::ZERO);
        let mut angle = if projected_up == Vec3::ZERO {
            0.0
        } else {
            acosf(radial.dot(projected_up).clamp(-1.0, 1.0))
        };
        if radial.cross(projected_up).dot(side) < 0.0 {
            angle = -angle;
        }
        let min_end = angle + rate * t.min_duration;
        let max_end = angle + rate * t.max_duration;
        let end_angle = min_end.max(t.end_angle * (PI / 180.0)).min(max_end);
        let delta = end_angle - angle;
        p.duration = delta / rate;
        let end_radial = rotate(radial, axis, delta);
        let end = contact + end_radial * t.end_leg_length;
        let middle = (end_radial + radial).normalize_or(radial);
        p.release =
            (vel - middle * vel.dot(middle)) * multiplier + middle * t.release_outward_speed;
        p.curve = [
            com,
            com + vel * (multiplier * p.duration * t.start_handle),
            end - p.release * (p.duration * t.end_handle),
            end,
        ];
        p
    }

    /// The centre of mass on the curve at share `s` (0..1) of the swing.
    pub fn com_at(&self, s: f32) -> Vec3 {
        let s = s.clamp(0.0, 1.0);
        let u = 1.0 - s;
        let [a, b, c, d] = self.curve;
        a * (u * u * u) + b * (3.0 * s * u * u) + c * (3.0 * s * s * u) + d * (s * s * s)
    }

    /// One tick (SK8 ground.rs `update`): the centre of mass now, and whether the swing is over (it flies
    /// off with [`AirPlant::release`]).
    pub fn step(&mut self, dt: f32) -> (Vec3, bool) {
        self.elapsed += dt;
        let s = if self.duration > 0.0 {
            self.elapsed / self.duration
        } else {
            1.0
        };
        (self.com_at(s), self.elapsed > self.duration)
    }

    /// Seconds of the swing (0 when it is released at once): what `FootPlantAbsorb` hands the absorb
    /// clips (`absorblength`).
    pub fn length(&self) -> f32 {
        self.duration.max(0.0)
    }
}

/// SK8 ground.rs `post_physics` (TU3 `0x82D4C6D8`): planted, too fast down or across wipes out.
pub fn plant_wipeout(t: &PlantTuning, vel: Vec3) -> Option<&'static str> {
    let descending = -vel.y;
    if descending > t.max_descending_speed {
        return Some("came down onto the planted foot too fast");
    }
    if vel.horizontal().length() > t.max_horizontal_speed {
        return Some("too fast across the planted foot");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::{Park, Shape};
    use alloc::vec;
    use alloc::vec::Vec;

    const T: PlantTuning = PlantTuning::BUILT_IN;

    #[test]
    fn the_skate_3_table_picks_the_plant() {
        // Goofy (not mirrored): left trigger is the front hand.
        assert_eq!(
            PlantKind::from_grab(true, true, false),
            PlantKind::FsBoneless
        );
        assert_eq!(
            PlantKind::from_grab(true, false, false),
            PlantKind::FsFastplant
        );
        assert_eq!(
            PlantKind::from_grab(false, true, false),
            PlantKind::BsBoneless
        );
        assert_eq!(
            PlantKind::from_grab(false, false, false),
            PlantKind::BsFastplant
        );
        // Regular (mirrored).
        assert_eq!(
            PlantKind::from_grab(true, false, true),
            PlantKind::BsBoneless
        );
        assert_eq!(
            PlantKind::from_grab(true, true, true),
            PlantKind::BsFastplant
        );
        assert_eq!(
            PlantKind::from_grab(false, false, true),
            PlantKind::FsBoneless
        );
        assert_eq!(
            PlantKind::from_grab(false, true, true),
            PlantKind::FsFastplant
        );
        for k in [
            PlantKind::FsBoneless,
            PlantKind::BsBoneless,
            PlantKind::FsFastplant,
            PlantKind::BsFastplant,
            PlantKind::NoComply,
        ] {
            assert_eq!(PlantKind::from_trick(k.trick()), Some(k));
        }
        assert_eq!(PlantKind::from_trick("Ollie"), None);
    }

    #[test]
    fn the_foot_jump_on_flat_jumps_three_metres_a_second_up() {
        let fwd = v3(0.0, 0.0, 1.0);
        // Rolling at 6 m/s: the planted share is all of it on flat (blend 1 at normal y 1).
        let v = foot_jump(&T, Vec3::UP, fwd * 6.0, fwd * 6.0, fwd);
        assert!((v.y - 3.0).abs() < 1e-4, "{v:?}");
        let k = T.jump_speed_scale.eval(6.0);
        assert!((v.z - 6.0 * k).abs() < 1e-4, "{v:?}");
        assert!(v.z < 6.0, "a boneless keeps less than the speed: {v:?}");
        // Standing still it still moves off along the facing.
        let v = foot_jump(&T, Vec3::UP, Vec3::ZERO, Vec3::ZERO, fwd);
        assert!((v.z - JUMP_MIN_FORWARD).abs() < 1e-4, "{v:?}");
        assert!((v.y - 3.0).abs() < 1e-4);
        // On a steep bank less goes up the normal.
        let n = v3(0.0, 0.5, -0.866_025_4);
        let v = foot_jump(&T, n, Vec3::ZERO, Vec3::ZERO, fwd);
        assert!(v.dot(n) < 2.5, "{v:?}");
    }

    #[test]
    fn the_swing_goes_over_the_foot_and_flies_off_along_it() {
        // Coming up at a wall (normal -z), the foot planted ahead of the rider at its face.
        let contact = v3(0.0, 1.0, 5.0);
        let com = v3(0.0, 1.0, 5.0 - 0.9);
        let vel = v3(0.0, 4.0, 4.0);
        let p = AirPlant::start(&T, com, vel, contact, v3(0.0, 0.0, -1.0));
        assert!(
            p.duration >= T.min_duration - 1e-4 && p.duration <= T.max_duration + 1e-4,
            "{p:?}"
        );
        assert_eq!(p.com_at(0.0), com);
        let end = p.com_at(1.0);
        assert!(
            ((end - contact).length() - T.end_leg_length).abs() < 1e-3,
            "{end:?}"
        );
        assert!(end.y > com.y, "swung up: {end:?}");
        assert!(p.release.y > vel.y * 0.5, "{p:?}");
        assert!(p.release.z < 0.5, "off the wall, not into it: {p:?}");
        let mut q = p;
        let mut done = false;
        for _ in 0..30 {
            if q.step(STEP).1 {
                done = true;
                break;
            }
        }
        assert!(done);
        assert!(plant_wipeout(&T, p.release).is_none());
        assert!(plant_wipeout(&T, v3(0.0, -12.0, 0.0)).is_some());
        assert!(plant_wipeout(&T, v3(12.0, 0.0, 0.0)).is_some());
    }

    #[test]
    fn the_manager_takes_up_a_contact_in_its_window_and_plants_on_it() {
        // A block from z = 3 ahead, 1 m high; flying up over its edge at 5 m/s.
        let park = Park {
            shapes: vec![Shape::Block {
                min: v3(-3.0, 0.0, 3.0),
                max: v3(3.0, 1.0, 6.0),
                rgba: 0,
            }],
            half_size: 30.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        };
        let mut m = Manager::default();
        let mut com = v3(0.0, 1.9, 0.0);
        let mut vel = v3(0.0, 3.0, 5.0);
        let mut planted = None;
        let mut taken = false;
        for i in 0..90 {
            let f = Frame {
                com,
                vel,
                gravity: 9.8,
                up: Vec3::UP,
                landing_normal: Vec3::UP,
                landing_time: 1.0,
                candidate: true,
                planting: m.active,
            };
            m.update(&T, &park, &f);
            taken |= m.active;
            if m.perform {
                planted = Some(i);
                break;
            }
            com += vel * STEP;
            vel.y -= 9.8 * STEP;
        }
        assert!(taken, "{m:?}");
        assert!(planted.is_some(), "{m:?}");
        // The foot reached down-forward onto the block's top.
        assert!(m.normal.y > 0.9, "{m:?}");
        assert!(m.contact.z > 3.0, "{m:?}");
        assert!(m.published_contact() <= PERFORM_TIME);
    }
}
