//! What the camera reads of the skater each tick (SK8 skate-game camera/publication.rs, skate-core
//! camera/anchors.rs and subject.rs): the board's frame, the body's reference points, the anchors the
//! camera's arm hangs from, the predicted flight, and the flags its shot choice reads.

use crate::math::{Vec3, v3};
use crate::park::Terrain;
use crate::skater::{Mode, Skater};

use super::shot::{
    REF_ANCHOR, REF_BOARD, REF_COM, REF_DAMPED_COM, REF_FEET, REF_GRIND, REF_HEAD, REF_HIPS,
    REF_TRAJECTORY, REFS,
};

/// The flight the board is on (from the landing prediction).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trajectory {
    pub launch: Vec3,
    pub launch_normal: Vec3,
    pub landing: Vec3,
    pub landing_normal: Vec3,
    pub apex: Vec3,
    /// Seconds from the flight's start to its top, since its start, and in all.
    pub apex_time: f32,
    pub time: f32,
    pub duration: f32,
    /// The flight's direction (horizontal, unit).
    pub dir: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subject {
    /// The board's position and its travel frame: forward (the nose, turned round riding fakie), up.
    pub pos: Vec3,
    pub fwd: Vec3,
    pub up: Vec3,
    /// The body's up (what the look-at height is measured along).
    pub offset_dir: Vec3,
    pub vel: Vec3,
    pub refs: [Vec3; REFS],
    /// The ground's up where the board last rode, and under it now.
    pub ground_up: Vec3,
    pub ground_normal: Vec3,
    pub trajectory: Option<Trajectory>,
    /// The air without a known landing yet: how long it has gone on.
    pub grinding: bool,
    pub ground: bool,
    pub wiping_out: bool,
    pub off_board: bool,
    /// Off the board in the air (the walker's jump or a fall).
    pub air_off_board: bool,
    pub pushing: bool,
    pub at_pushable_speed: bool,
    pub pumping: f32,
    pub grind_dir: Vec3,
    /// Steering, + left (the rider's own sense).
    pub steering: f32,
    /// The steering the turning clocks read (SK8 `steering_for_turn`): the physics' turn, or the
    /// stick's own turn or kick-turn input when it agrees in sign and is larger.
    pub steering_for_turn: f32,
    /// The rider's stance is mirrored against the shots' (switch, regular, rolling fakie).
    pub mirrored: bool,
    pub manual: bool,
    pub preparing: bool,
    pub onboard_air: bool,
    pub hippy: bool,
    /// Grinding a ledge whose top is on the travel's left / right.
    pub ledge_left: bool,
    pub ledge_right: bool,
    pub reset: bool,
}

/// The seven anchors (Skate 3's order: skateboard, hips, air, grind, world, damped COM, point of
/// interest) and their velocities.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchors {
    pub pos: [Vec3; 7],
    pub vel: [Vec3; 7],
    grind: Vec3,
}

impl Anchors {
    pub fn update(&mut self, s: &Subject, dt: f32) {
        let board = s.pos + s.offset_dir * 0.3;
        let air = s.refs[REF_COM] - s.offset_dir * 0.33;
        if s.reset || s.grinding {
            let p = if s.reset { s.pos } else { s.refs[REF_GRIND] };
            self.grind = p + s.offset_dir * 0.5;
        } else {
            self.grind += s.vel * dt;
        }
        let pos = [
            board,
            s.refs[REF_COM],
            air,
            self.grind,
            Vec3::ZERO,
            s.refs[REF_DAMPED_COM],
            Vec3::ZERO,
        ];
        for ((p, v), new) in self.pos.iter_mut().zip(&mut self.vel).zip(pos) {
            *v = if s.reset || dt <= 0.0 {
                Vec3::ZERO
            } else {
                (new - *p) * (1.0 / dt)
            };
            *p = new;
        }
    }
}

/// Which side of the travel `dir` a ledge's top (its `high` side) is on: (left, right). Skate 3 marks
/// ledge shots with painted world regions; flatground reads the rail's high side. Left is where yaw
/// grows (up x travel).
pub fn ledge_side(dir: Vec3, high: Vec3) -> (bool, bool) {
    let left = Vec3::UP.cross(dir).normalize_or(Vec3::ZERO);
    let side = high.dot(left);
    (side > 0.3, side < -0.3)
}

/// What the subject keeps between ticks: the take-off, the damped centre of mass, the ground's last up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubjectHistory {
    takeoff: Option<(Vec3, Vec3)>,
    damped_com: Option<Vec3>,
    ground_up: Vec3,
    body_up: Vec3,
    grind_dir: Vec3,
}

impl Default for SubjectHistory {
    fn default() -> Self {
        SubjectHistory {
            takeoff: None,
            damped_com: None,
            ground_up: Vec3::UP,
            body_up: Vec3::UP,
            grind_dir: v3(0.0, 0.0, 1.0),
        }
    }
}

impl SubjectHistory {
    pub fn read(&mut self, s: &Skater, park: &dyn Terrain, reset: bool, tracked: Vec3) -> Subject {
        let airborne = s.is_airborne();
        let on_ground = matches!(s.mode, Mode::Rolling | Mode::Grind);
        if on_ground {
            self.ground_up = s.contact_n.normalize_or(Vec3::UP);
            self.takeoff = None;
        } else if airborne && self.takeoff.is_none() {
            self.takeoff = Some((s.pos, self.ground_up));
        }
        // Travel forward: the nose, turned round when rolling tail first.
        let nose = s.fwd.normalize_or(v3(0.0, 0.0, 1.0));
        let fwd = if s.fakie || (on_ground && s.speed < -0.1) {
            -nose
        } else {
            nose
        };
        let trajectory = match (s.mode, &s.plan) {
            (Mode::Air { time }, Some(p)) => {
                let (launch, launch_normal) =
                    self.takeoff.unwrap_or((p.flight.pos, self.ground_up));
                let g = p.flight.gravity.max(0.1);
                let apex_time = (p.flight.vel.y / g).clamp(0.0, p.contact.time.max(0.0));
                let apex = p.flight.at(apex_time);
                let apex = if apex.y > launch.y { apex } else { launch };
                let h = p.flight.vel.horizontal();
                let dir = if h.length() > 0.05 {
                    h.normalize_or(fwd)
                } else {
                    fwd.horizontal().normalize_or(v3(0.0, 0.0, 1.0))
                };
                Some(Trajectory {
                    launch,
                    launch_normal,
                    landing: p.contact.point,
                    landing_normal: p.normal.normalize_or(Vec3::UP),
                    apex,
                    apex_time,
                    time: (time - p.start).max(0.0),
                    duration: p.contact.time.max(1e-3),
                    dir,
                })
            }
            _ => None,
        };
        // The body's up: the ground's on it, in the air the take-off's turning to the landing's.
        let body_up = match (&trajectory, s.mode) {
            (Some(t), _) => t
                .launch_normal
                .lerp(t.landing_normal, (t.time / t.duration).clamp(0.0, 1.0))
                .normalize_or(Vec3::UP),
            (None, Mode::Rolling | Mode::Grind) => s.normal.normalize_or(Vec3::UP),
            (None, Mode::Air { .. }) => self.body_up,
            _ => Vec3::UP,
        };
        self.body_up = body_up;
        let up = body_up;
        let crouch = s.crouch.clamp(0.0, 1.0);
        let com = s.pos + up * (0.95 - 0.3 * crouch);
        let head = s.pos + up * (1.62 - 0.4 * crouch);
        let feet = s.pos + up * 0.08;
        let damped = match self.damped_com {
            Some(d) if !reset && (d - com).length() < 3.0 => d + (com - d) * 0.25,
            _ => com,
        };
        self.damped_com = Some(damped);
        let grind_point = s.grind.as_ref().map_or(s.pos, |g| g.point);
        let mut refs = [Vec3::ZERO; REFS];
        refs[REF_HEAD] = head;
        refs[REF_HIPS] = com;
        refs[REF_GRIND] = if s.mode == Mode::Grind {
            grind_point
        } else {
            s.pos
        };
        refs[REF_COM] = com;
        refs[REF_BOARD] = s.pos;
        refs[REF_TRAJECTORY] = com;
        refs[REF_ANCHOR] = tracked;
        refs[REF_FEET] = feet;
        refs[REF_DAMPED_COM] = damped;
        let mut ledge_left = false;
        let mut ledge_right = false;
        if let Some(g) = &s.grind {
            let rails = park.rails();
            if let Some(r) = rails.get(g.rail) {
                let d = r.at(g.s).1 * g.dir;
                self.grind_dir = d.normalize_or(fwd);
                if g.class.is_ledge() {
                    (ledge_left, ledge_right) = ledge_side(d, g.high);
                }
            }
        }
        // On a rail the travel is along the rail (a boardslide's nose points across it).
        let fwd = if s.mode == Mode::Grind {
            self.grind_dir
        } else {
            fwd
        };
        let vel = match s.mode {
            Mode::Rolling => s.ground_velocity(),
            _ => s.vel,
        };
        Subject {
            pos: s.pos,
            fwd,
            up: s.normal.normalize_or(Vec3::UP),
            offset_dir: up,
            vel,
            refs,
            ground_up: self.ground_up,
            ground_normal: if on_ground {
                s.contact_n.normalize_or(Vec3::UP)
            } else {
                Vec3::UP
            },
            trajectory,
            grinding: s.mode == Mode::Grind,
            ground: s.mode == Mode::Rolling,
            wiping_out: matches!(s.mode, Mode::Bailed { .. }),
            off_board: matches!(s.mode, Mode::Runout { .. } | Mode::Walk) || s.hippie_air(),
            air_off_board: s.mode == Mode::Walk && s.biped.air.is_some(),
            pushing: s.push.is_some(),
            at_pushable_speed: s.speed.abs() < s.tuning.push_max_speed,
            pumping: if s.pump.time > 0.0 && s.pump.turn_rate > 0.0 {
                s.pump.effect * 60.0
            } else {
                0.0
            },
            grind_dir: self.grind_dir,
            steering: s.steer,
            steering_for_turn: steering_for_turn(s.steer, s.turn_input, s.kick_input),
            mirrored: s.mirrored() != s.fakie,
            manual: s.manual.is_some(),
            // Skate 3: the playing clips' `Anticipating` attribute (the ollie wind-up); without the
            // graphs, a crouch.
            preparing: s.mode == Mode::Rolling
                && s.graph.map_or(s.crouch > 0.35, |g| g.anticipating),
            onboard_air: airborne && !s.hippie_air(),
            hippy: s.hippie_air(),
            ledge_left,
            ledge_right,
            reset,
        }
    }
}

/// SK8 manager_subject.rs `steering_for_turn`: the larger of the stick's turn and kick-turn inputs,
/// taken over the physics' turn when it has the same sign and is larger.
pub fn steering_for_turn(primary: f32, turn: f32, kick: f32) -> f32 {
    let selected = if kick.abs() > turn.abs() { kick } else { turn };
    let sign = |v: f32| {
        if v > 0.0 {
            1
        } else if v < 0.0 {
            -1
        } else {
            0
        }
    };
    if sign(selected) != sign(primary) || selected.abs() < primary.abs() {
        primary
    } else {
        selected
    }
}
