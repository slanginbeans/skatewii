//! The gameplay camera, after Skate 3's (SK8-ENGINE/skate-3-rust-engine, GPL-3.0,
//! crates/skate-core/src/camera and crates/skate-game/src/camera): each 1/60 s tick
//!
//! 1. reads the skater ([`subject`]): the board's travel frame, the body's reference points, the
//!    anchors, the predicted flight;
//! 2. updates what the shot choice reads ([`rig::State`]: the flight's height and air time, slopes,
//!    stance, steering clocks) and the compass the shots turn with ([`rig::Compass`]);
//! 3. chooses a shot ([`select`]): Skate 3's camera graph when the disc supplied it, else flatground's
//!    own choice;
//! 4. runs the shot tree and the transition from the previous shot ([`shot`]), binds the shot to the
//!    arm (heading, elevation, distance, drop and collision raises) and runs the rig ([`rig::Rig`]):
//!    heading and elevation trackers, the anchor's push lag, the look-at height through the air,
//!    avoidance, collision;
//! 5. frames the skater ([`frame`]): framing yaw, pitch and roll eased between shots, then pan and tilt
//!    trackers;
//! 6. shakes the view ([`shake`]): a sway with speed and an impulse on landings.
//!
//! The arm also takes the free look's offset ([`look`]: the look stick, in shots that allow it), the
//! player's High or Low camera chooses which half of the graph runs ([`select::HIGH`], [`select::LOW`]),
//! and the graph's slow motion on long airs asks for a slower simulation clock ([`slow_motion`],
//! [`CameraMan::rate`]).
//!
//! Skate 3's shots, graph and shake samples come from the player's disc (`rules/camera/shots`,
//! `rules/camera/graph`, `rules/camera/shake`). The tests stand in for them (the `fixtures` feature, off
//! in what ships) with synthetic shots, a simple choice and a sine shake loop (`builtin`).

#[cfg(any(test, feature = "fixtures"))]
pub mod builtin;
pub mod frame;
pub mod look;
pub mod rig;
pub mod select;
pub mod shake;
pub mod shot;
pub mod slow_motion;
pub mod subject;
pub mod tracker;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::gfx::Camera;
use crate::math::{Vec3, atan2f, sqrtf, v3};
use crate::park::Terrain;
use crate::skater::Skater;
use crate::tuning::CamTuning;

use frame::{Composer, Orientation};
use look::LookInput;
use rig::{Compass, DropPredictor, Rig, STEP, State, bind_shot};
use select::{CamGraph, Choice, Conditions};
use shake::{Samples, Shake};
use shot::{BlendInputs, OPT_DROP, OPT_LOOK, Shot, ShotManager, ShotSet};
use slow_motion::SlowMotion;
use subject::{Anchors, Subject, SubjectHistory};
use tracker::RAD;

/// Shots, the graph that chooses them and the shake's samples.
#[derive(Clone, Debug)]
pub struct CamData {
    pub shots: ShotSet,
    pub graph: Option<CamGraph>,
    pub shake: Samples,
}

impl Default for CamData {
    /// Nothing loaded yet (the rules pack brings the disc's; the unit tests their synthetic stand-ins).
    fn default() -> Self {
        #[cfg(any(test, feature = "fixtures"))]
        return CamData {
            shots: ShotSet::built_in(),
            graph: None,
            shake: Samples::built_in(),
        };
        #[cfg(not(any(test, feature = "fixtures")))]
        CamData {
            shots: ShotSet {
                defs: Vec::new(),
                imported: false,
            },
            graph: None,
            shake: Samples::still(),
        }
    }
}

/// The deck's height over where the wheels touch (m).
const DECK: f32 = 0.1;
/// Seconds between two shot-change lines in the log.
const LOG_EVERY: f32 = 0.25;
/// A skater further than this (m) from where it was a tick ago was teleported: the camera resets.
const TELEPORT: f32 = 4.0;

/// Skate 3's vertical field of view for a lens (`FramingLensLength`, mm on a 35 mm frame) and a screen
/// aspect (SK8 manager_frame.rs): the frame's angle over the square root of aspect + 1.
pub fn lens_fov(lens: f32, aspect: f32) -> f32 {
    let deg = 2.0 * atan2f(35.0, 2.0 * lens.max(1.0)) / RAD;
    sqrtf(deg * deg / (aspect.max(0.5) + 1.0))
}

pub struct CameraMan {
    pub data: CamData,
    pub tuning: CamTuning,
    pub shots: ShotManager,
    pub state: State,
    pub compass: Compass,
    anchors: Anchors,
    history: SubjectHistory,
    pub rig: Rig,
    pub drop: DropPredictor,
    composer: Composer,
    pub orient: Orientation,
    heading_mirror: f32,
    framing_mirror: f32,
    /// Screen width over height.
    pub aspect: f32,
    reset: bool,
    instant_frames: u32,
    last_pos: Option<Vec3>,
    /// The chosen shot's name and the conditions it was chosen from.
    pub shot_name: String,
    pub conditions: Conditions,
    pub subject: Option<Subject>,
    pending_log: Option<String>,
    since_log: f32,
    log: Vec<String>,
    eye: Vec3,
    at: Vec3,
    up: Vec3,
    /// The player's camera: [`select::LOW`] (Skate 3's constructor default) or [`select::HIGH`].
    pub camera_type: u32,
    /// The look stick this tick (each axis -1..1; the game hands it over, [0, 0] when not looking), and
    /// the arm's offset it makes in shots that allow it.
    pub look_stick: [f32; 2],
    pub look: LookInput,
    pub shake: Shake,
    /// The drawn view: the rig's shaken.
    shaken: (Vec3, Vec3, Vec3),
    /// The built-in choice's slow motion while it runs (the graph keeps its own), and the simulation
    /// rate the camera asks for (ticks a second).
    slow: Option<SlowMotion>,
    rate: f32,
}

impl CameraMan {
    pub fn new(s: &Skater, park: &dyn Terrain, data: CamData, tuning: CamTuning) -> CameraMan {
        let mut c = CameraMan {
            data,
            tuning,
            shots: ShotManager::new(),
            state: State::default(),
            compass: Compass::default(),
            anchors: Anchors::default(),
            history: SubjectHistory::default(),
            rig: Rig::default(),
            drop: DropPredictor::default(),
            composer: Composer::default(),
            orient: Orientation::default(),
            heading_mirror: 1.0,
            framing_mirror: 1.0,
            aspect: 4.0 / 3.0,
            reset: true,
            instant_frames: 3,
            last_pos: None,
            shot_name: String::new(),
            conditions: Conditions::default(),
            subject: None,
            pending_log: None,
            since_log: LOG_EVERY,
            log: Vec::new(),
            eye: s.pos,
            at: v3(0.0, 0.0, 1.0),
            up: Vec3::UP,
            camera_type: select::LOW,
            look_stick: [0.0; 2],
            look: LookInput::default(),
            shake: Shake::default(),
            shaken: (s.pos, v3(0.0, 0.0, 1.0), Vec3::UP),
            slow: None,
            rate: slow_motion::NORMAL,
        };
        c.update(s, park, STEP);
        c
    }

    /// Cuts straight to the shot on the next tick (the skater was put somewhere else).
    pub fn request_reset(&mut self) {
        self.reset = true;
    }

    /// The simulation's ticks a second the camera asks for: 60, or fewer through the slow motion of a
    /// long air.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// The slow motion of a long air is running (Skate 3's `SlowMotionController`).
    pub fn slowing(&self) -> bool {
        self.slow.is_some() || self.data.graph.as_ref().is_some_and(|g| g.slowing())
    }

    /// The free look is turning the camera now (the current shot reads the look stick and it is off
    /// centre).
    pub fn looking(&self) -> bool {
        self.look_allowed() && self.look_stick != [0.0, 0.0]
    }

    /// The current shot reads the look stick (Skate 3's `OptionUseFreeCamStick`).
    pub fn look_allowed(&self) -> bool {
        self.shots.current_shot().has(OPT_LOOK)
    }

    /// New shots, graph or tuning (keeps the aspect); the camera cuts to its shot on the next tick.
    pub fn set_data(&mut self, data: CamData, tuning: CamTuning) {
        self.data = data;
        self.tuning = tuning;
        self.shots = ShotManager::new();
        self.reset = true;
        self.instant_frames = 3;
    }

    /// Shot-change lines for the development log.
    pub fn take_log(&mut self) -> Vec<String> {
        core::mem::take(&mut self.log)
    }

    fn choose(&mut self, c: &Choice, inputs: &BlendInputs) {
        let Some(def) = self.data.shots.find(&c.name) else {
            let line = format!("camera: no shot {}", c.name);
            if !self.log.contains(&line) && self.log.len() < 16 {
                self.log.push(line);
            }
            return;
        };
        if self.shots.choose(&self.data.shots, def, false, inputs) {
            self.shot_name = c.name.clone();
            let state = self
                .data
                .graph
                .as_ref()
                .map_or(String::from("built-in"), |g| g.state());
            self.pending_log = Some(format!("camera: {} ({state})", c.name));
            // Units 2 (a speed): how far the camera has to travel to the new shot.
            let shot = self.shots.current_shot();
            let a = self.rig.anchor;
            let target = a + rig::arm(shot.arm_elevation, shot.arm_heading) * shot.distance;
            self.shots.transition_distance = (target - self.rig.positioner.position).length();
        }
        self.shots
            .graph_transition(c.transition_in, c.transition_out);
    }

    pub fn update(&mut self, s: &Skater, park: &dyn Terrain, dt: f32) {
        let t = self.tuning;
        let teleported = self
            .last_pos
            .is_some_and(|p| (s.pos - p).length() > TELEPORT);
        let reset = self.reset || teleported || self.last_pos.is_none();
        self.reset = false;
        self.last_pos = Some(s.pos);
        if reset {
            self.instant_frames = 3;
            self.drop.reset();
            if let Some(g) = &mut self.data.graph {
                g.restart();
            }
        }
        let subj = self.history.read(s, park, reset, self.rig.tracked.pos);
        self.anchors.update(&subj, STEP);
        self.rig.anchors_pos = self.anchors.pos;
        let camera = self.rig.positioner.position;

        // What the shot choice reads.
        self.state.update_timing(&subj, &t);
        let ground = self.rig.mode == 2 || subj.trajectory.is_none();
        if let Some((normal, vel)) = self.state.update_air(&subj, &mut self.rig, ground, camera) {
            self.shake.landing(normal, vel, &t.shake);
        }
        self.state.update_incline(&subj);
        let current = self.shots.current_shot();
        self.compass.update(
            &subj,
            &t,
            camera,
            if reset { 0.0 } else { dt },
            current.compass,
        );

        let inputs = self.blend_inputs(&subj);
        self.conditions = self.conditions(&subj);
        let choices = match &mut self.data.graph {
            Some(g) => {
                let c = g.update(&self.conditions, dt, &t);
                self.rate = g.rate;
                c
            }
            // The unit tests' stand-in for the disc's graph.
            #[cfg(any(test, feature = "fixtures"))]
            None => {
                self.built_in_slow(dt);
                alloc::vec![select::built_in(&self.conditions, &self.data.shots)]
            }
            #[cfg(not(any(test, feature = "fixtures")))]
            None => Vec::new(),
        };
        if reset {
            self.shots.instant = true;
        }
        for c in &choices {
            self.choose(c, &inputs);
        }

        // The shot onto the rig.
        let mirror_shot = if reset {
            self.shots.first_leaf()
        } else {
            self.shots.current_shot()
        };
        self.state.update_mirrors(reset, ground, &mirror_shot);
        let drop_on = self.shots.current_shot().has(OPT_DROP) && !reset;
        let speed = self.rig.velocity.length();
        let vel = if reset || speed < t.drop_min_test {
            subj.fwd * t.drop_min_test
        } else {
            self.rig.velocity
        };
        // From deck height (the board's position is where its wheels touch).
        let deck = subj.pos + subj.offset_dir * DECK;
        self.drop
            .update(park, deck, vel, drop_on, subj.off_board, &t);
        // Free look: the stick as the shot lets the camera read it (SK8 manager.rs `look_enabled`).
        let look_on = if reset {
            self.shots.first_leaf()
        } else {
            self.shots.current_shot()
        }
        .has(OPT_LOOK);
        self.look
            .update(if look_on { self.look_stick } else { [0.0; 2] }, &t);
        if reset {
            self.shots.make_instant();
            self.rig.velocity = Vec3::ZERO;
            self.rig.acceleration = Vec3::ZERO;
            self.shake = Shake::default();
        }
        let inputs = self.blend_inputs(&subj);
        self.shots.update(&self.data.shots, dt, &inputs);
        // The anchor.
        let anchor = if reset {
            self.shots.interpolated.anchor
        } else {
            self.shots.current_shot().anchor
        }
        .min(6);
        if !reset && anchor != self.state.selected_anchor {
            self.rig.rebind = true;
        }
        self.state.selected_anchor = anchor;
        self.state.anchor = self.anchors.pos[anchor as usize];
        self.state.anchor_vel = if reset {
            Vec3::ZERO
        } else {
            self.anchors.vel[anchor as usize]
        };
        self.rig.acceleration = self.state.anchor_vel - self.rig.velocity;
        self.rig.velocity = self.state.anchor_vel;
        self.rig.anchor = self.state.anchor;
        let interpolated = self.shots.interpolated;
        let transitioning = self.shots.transitioning();
        let (hm, fm) = bind_shot(
            &mut self.state,
            &mut self.rig,
            &interpolated,
            &subj,
            &self.drop,
            self.look.elevation,
            true,
            reset,
            ground,
            transitioning,
        );
        self.heading_mirror = hm;
        self.framing_mirror = fm;
        let yaw = 1.0 - interpolated.smoothing[2];
        let pitch = 1.0 - interpolated.smoothing[3];
        self.orient.pan_smoothing = 1.0 - yaw * yaw;
        self.orient.tilt_smoothing = 1.0 - pitch * pitch;
        self.orient.roll = self.composer.roll;
        self.orient.framing_mirror = fm;
        if reset {
            self.rig.teleport();
        }
        let min_distance = if subj.wiping_out {
            t.min_distance_wipeout
        } else {
            t.min_distance
        };
        self.rig.update(
            park,
            &subj,
            &t,
            dt,
            min_distance,
            interpolated.collision != 2,
        );
        let current = self.shots.current_shot();
        let target = self.composer.compose(
            dt,
            self.shots.framing_progress(),
            fm,
            &self.shots.previous,
            &current,
            self.rig.tracked.pos,
            self.rig.positioner.position,
            &subj.refs,
            subj.offset_dir,
            &t,
        );
        let snap = reset || self.rig.reangle;
        self.rig.reangle = false;
        self.orient
            .update(dt, target, &t, snap, self.rig.follow_in_air);
        self.shots.instant = reset || self.instant_frames > 0;
        if reset {
            self.state.fresh = true;
        } else if ground {
            self.state.fresh = false;
        }
        self.instant_frames = self.instant_frames.saturating_sub(1);
        self.eye = self.rig.positioner.position;
        let (at, up) = self.orient.view();
        self.at = at;
        self.up = up;
        // The shake: by the camera's speed over the ground and its arm's length (SK8 manager_output.rs).
        let v = self.rig.velocity;
        let speed = sqrtf(v.x * v.x + v.z * v.z);
        let arm = (self.rig.anchor - self.eye).length();
        let offset =
            self.shake
                .update(dt, speed, arm, !subj.wiping_out, &self.data.shake, &t.shake);
        self.shaken = shaken(self.eye, at, up, offset);
        self.subject = Some(subj);
        // The log, at most one shot line every quarter second.
        self.since_log += dt;
        if self.since_log >= LOG_EVERY
            && let Some(line) = self.pending_log.take()
        {
            self.since_log = 0.0;
            if self.log.len() < 64 {
                self.log.push(line);
            }
        }
    }

    /// The unit tests' stand-in for Skate 3's graph: its slow-motion state's condition, run as the
    /// graph would.
    #[cfg(any(test, feature = "fixtures"))]
    fn built_in_slow(&mut self, dt: f32) {
        let on = select::built_in_slow(&self.conditions);
        match (&mut self.slow, on) {
            (None, true) => {
                let (s, rate) = SlowMotion::begin(&self.tuning);
                self.slow = Some(s);
                self.rate = rate;
            }
            (Some(s), true) => self.rate = s.update(dt, self.conditions.air_time, &self.tuning),
            (Some(_), false) => {
                self.slow = None;
                self.rate = slow_motion::NORMAL;
            }
            (None, false) => self.rate = slow_motion::NORMAL,
        }
    }

    fn blend_inputs(&self, s: &Subject) -> BlendInputs {
        let st = &self.state;
        let air_time = s.trajectory.map_or(0.0, |t| t.duration);
        let (air_progress, apex_progress) = match s.trajectory {
            Some(t) if !(self.rig.mode == 2) => {
                let air = if st.apex_height <= 0.0 {
                    0.0
                } else {
                    (t.time / t.duration).clamp(0.0, 1.0)
                };
                let apex = if st.apex_height <= 0.0 || t.duration < st.apex_time {
                    0.0
                } else {
                    let twice = st.apex_time * 2.0;
                    if t.time > twice {
                        ((t.time - twice) / (t.duration - twice).max(1e-3) + 1.0).clamp(1.0, 2.0)
                    } else {
                        t.time / twice.max(1e-3)
                    }
                };
                (air, apex)
            }
            _ => (1.0, if st.was_ground { 0.0 } else { 1.0 }),
        };
        BlendInputs {
            speed: self.rig.velocity.dot(s.fwd).abs(),
            accel: self.rig.acceleration.dot(s.fwd),
            apex_height: st.apex_height,
            air_progress,
            incline: st.velocity_incline / RAD,
            previous_distance: self.shots.previous.distance,
            apex_progress,
            landing_incline: st.landing_incline / RAD,
            steering: if s.mirrored { -s.steering } else { s.steering },
            frontside: st.frontside,
            air_time,
            direction_incline: st.direction_incline / RAD,
            mirror: self.heading_mirror,
            compass: self.compass.headings,
        }
    }

    fn conditions(&self, s: &Subject) -> Conditions {
        let st = &self.state;
        Conditions {
            wiping_out: s.wiping_out,
            wipe_tweak: s.wipe_tweak,
            off_board: s.off_board,
            air_off_board: s.air_off_board,
            onboard_air: s.onboard_air,
            preparing: s.preparing,
            manual: s.manual,
            hippy_jump: s.hippy,
            grinding: self.rig.mode == 2,
            was_grinding: self.rig.previous_mode == 2,
            on_ground: self.rig.mode == 0,
            reentry: st.reentry,
            ledge_left: s.ledge_left,
            ledge_right: s.ledge_right,
            speed: self.rig.velocity.dot(s.fwd),
            speed_y: self.rig.velocity.y,
            accel: self.rig.acceleration.dot(s.fwd),
            air_time: s.trajectory.map_or(0.0, |t| t.duration),
            landing_height: st.landing_delta,
            max_height: st.apex_height,
            height_ratio: st.apex_ratio,
            frontside: st.frontside,
            transfer: st.transfer / RAD,
            launch_incline: st.launch_incline / RAD,
            landing_incline: st.landing_incline / RAD,
            grind_incline: st.direction_incline / RAD,
            incline: st.velocity_incline / RAD,
            centred_time: st.centred_time,
            turning_time: st.steering_time,
            shot: self.shots.current.map(|i| self.data.shots.defs[i].key),
            camera_type: self.camera_type,
        }
    }

    /// The camera to draw with (shaken).
    pub fn camera(&self) -> Camera {
        let (eye, at, up) = self.shaken;
        Camera {
            eye,
            target: eye + at,
            up,
            fov_y_deg: lens_fov(self.shots.interpolated.lens, self.aspect),
            near: 0.1,
            far: 120.0,
        }
    }

    /// The camera without its shake (what the walker's stick is read against).
    pub fn steady(&self) -> Camera {
        Camera {
            eye: self.eye,
            target: self.eye + self.at,
            up: self.up,
            ..self.camera()
        }
    }

    /// The boom length right now (after any wall pull-in).
    pub fn distance(&self) -> f32 {
        (self.eye - self.rig.tracked.pos).length()
    }

    /// The interpolated shot being shown.
    pub fn shown(&self) -> Shot {
        self.shots.interpolated
    }
}

/// The view `eye`, looking along `at` with `up`, turned and moved by a shake `offset` about and along its
/// own right, up and forward axes (SK8 `basis_from_angles`: pitch, yaw, roll).
fn shaken(eye: Vec3, at: Vec3, up: Vec3, o: shake::Offset) -> (Vec3, Vec3, Vec3) {
    if o.is_zero() {
        return (eye, at, up);
    }
    let f = at.normalize_or(v3(0.0, 0.0, 1.0));
    let right = up.cross(f).normalize_or(v3(1.0, 0.0, 0.0));
    let u = f.cross(right);
    let [p, y, r] = o.rotation;
    let (sp, cp) = (crate::math::sinf(p), crate::math::cosf(p));
    let (sy, cy) = (crate::math::sinf(y), crate::math::cosf(y));
    let (sr, cr) = (crate::math::sinf(r), crate::math::cosf(r));
    // The rotation's up and forward columns, in the view's axes.
    let col_up = [sy * sp * cr - cp * sr, sy * sp * sr + cp * cr, cy * sp];
    let col_at = [sy * cp * cr + sp * sr, sy * cp * sr - sp * cr, cy * cp];
    let world = |c: [f32; 3]| right * c[0] + u * c[1] + f * c[2];
    let [tx, ty, tz] = o.translation;
    (
        eye + world([tx, ty, tz]),
        world(col_at) * at.length(),
        world(col_up),
    )
}

#[cfg(test)]
mod tests;
