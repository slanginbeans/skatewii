//! The game's side of the graph runner: builds it from `motion.fgpak`, feeds it the pad and the skater
//! each tick, and times it.

use alloc::string::String;
use alloc::vec::Vec;

use super::host::{Runner, World};
use super::input::{Gates, Intentions};
use super::ops::phys;
use super::{Error, Pack, Str};
use crate::gesture::Flick;
use crate::gesture::Shape;
use crate::input::Pad;
use crate::math::Vec3;
use crate::motion::MotionDb;
use crate::motion::NameId;
use crate::skater::{AirTrick, Family, GRAPH_NAMES, GraphControl, Mode, Skater, Stance};
use crate::tuning::Tuning;

/// Motion-library names the driver reads from the playing clips.
#[derive(Clone, Copy, Debug, Default)]
struct Clips {
    push_contact: Option<NameId>,
    brake_contact: Option<NameId>,
    jump: Option<NameId>,
    trick_height: Option<NameId>,
    danger: Option<NameId>,
    /// The stance events (SK8 skater_animation/state.rs `apply_stance_events`): `MIRRORED` and
    /// `SWITCH` toggle the rider's mirror and switch flags when a playing clip crosses them (the
    /// fakie-to-switch clips `R_SWITCH_RIDE_*`, just after they start).
    mirrored: Option<NameId>,
    switch: Option<NameId>,
    /// Off-board clips' `OB_TRAJ`: Skate 3's physics walks the rider while it is on.
    ob_traj: Option<NameId>,
    /// ToggleBoard's throw clips letting go of the board (SK8 extended_attributes.rs: `OB_ReleaseBoard`,
    /// `OB_ThrowBoard`).
    release: Option<NameId>,
    throw: Option<NameId>,
    /// The ground foot plants' clips (SK8 extended_attributes.rs): the board in the clip's hands
    /// (`AnimSkateboard`) and the jump off the planted foot (`FootJump`).
    anim_skateboard: Option<NameId>,
    foot_jump: Option<NameId>,
}

pub struct Driver {
    pub runner: Runner<'static>,
    pub input: Intentions,
    recognised: Vec<(Str, f32)>,
    /// Microseconds the last tick took (input, both graphs, animation advance).
    pub cost_us: u32,
    /// Memory the parsed graphs take (bytes).
    pub memory: usize,
    /// Microseconds parsing the graphs and their operations took.
    pub load_us: u32,
    shown: String,
    clips: Clips,
    /// The playing clips carried the jump attribute last tick.
    jumping: bool,
    /// The playing clips carried `FOOTJUMP` last tick.
    foot_jumping: bool,
    /// The motion state's path last tick (for edges).
    last_path: String,
    /// The graph's string for each grind name the physics publishes (`skater::GRAPH_NAMES`: Skate 3's
    /// `IsGrinding` grindName values, `BF_` variants included).
    grinds: Vec<Str>,
    /// Ticks the physics has been riding while the motion graph stayed off the board.
    off_board_ticks: u32,
}

/// The motion graph is put back on the board once the physics has been riding this long while it stays
/// off it (a get-on that went wrong): longer than a mount clip's tail after the rider is on.
pub const OFF_BOARD_RESYNC: u32 = 30;

impl Driver {
    /// `entry`: the `motion/graph` pack entry. `sets`: gesture files by name (`skater`, `skaterls`, ...).
    /// `ops`: the `motion/graph/ops` entry (the operations typed at import), if the pack has it.
    pub fn new(
        entry: &'static [u8],
        ops: Option<&[u8]>,
        db: &MotionDb,
        sets: Vec<(&str, Vec<Shape>)>,
    ) -> Result<Driver, Error> {
        let pack = Pack::parse(entry)?;
        let typed = ops.and_then(|o| super::opcode::decode(o, entry.len()));
        let memory = pack.graphs.iter().map(|g| g.memory()).sum();
        let input = Intentions::new(&pack, sets);
        let grinds = GRAPH_NAMES
            .iter()
            .map(|n| pack.find(n).unwrap_or(super::NONE))
            .collect();
        let runner = Runner::with_ops(pack, Some(db), typed).ok_or(Error::Bad)?;
        Ok(Driver {
            runner,
            input,
            recognised: Vec::new(),
            cost_us: 0,
            memory,
            load_us: 0,
            shown: String::new(),
            clips: Clips {
                push_contact: db.name_id("PUSHCONTACT"),
                brake_contact: db.name_id("BRAKE_CONTACT"),
                jump: db.name_id("JUMP"),
                trick_height: db.name_id("TRICKHEIGHT"),
                danger: db.name_id("DANGERZONE"),
                mirrored: db.name_id("MIRRORED"),
                switch: db.name_id("SWITCH"),
                ob_traj: db.name_id("OB_TRAJ"),
                release: db.name_id("OB_RELEASEBOARD"),
                throw: db.name_id("OB_THROWBOARD"),
                anim_skateboard: db.name_id("ANIMSKATEBOARD"),
                foot_jump: db.name_id("FOOTJUMP"),
            },
            jumping: false,
            foot_jumping: false,
            last_path: String::new(),
            off_board_ticks: 0,
            grinds,
        })
    }

    pub fn set_gestures(&mut self, sets: Vec<(&str, Vec<Shape>)>) {
        self.input = Intentions::new(&self.runner.pack, sets);
    }

    /// The tuning the push behaviours read (`anim_motion/pushing`).
    pub fn set_tuning(&mut self, t: &Tuning) {
        self.runner.s.push_curves = super::push::PushCurves { tuning: *t };
    }

    /// What the graphs read of the skater.
    pub fn world(&self, s: &Skater, time_to_land: Option<f32>, idle: f32) -> World {
        let phys = match s.mode {
            Mode::Rolling => phys::GROUND,
            // A hippie jump is Skate 3's LandingOnDeck: the rider off the board above it (an off-board
            // physics state), the board rolling on underneath. Getting on in the air stays its BipedAir
            // (off-board air) until he is down on the board.
            Mode::Air { .. } if s.hippie_air() => phys::OFFBOARD,
            Mode::Air { .. } if s.landing_on_deck() => phys::OFFBOARD_AIR,
            Mode::Air { .. } => phys::AIR,
            Mode::Grind => phys::GRIND,
            Mode::Bailed { .. } | Mode::Runout { .. } => phys::WIPEOUT,
            Mode::Walk if s.biped.air.is_some() => phys::OFFBOARD_AIR,
            Mode::Walk => phys::OFFBOARD,
        };
        // Dropping in on foot: the board's tail held on the coping is Skate 3's tip slide with its
        // drop-in flag (a grind to the graphs).
        let phys = if s.drop_in.is_some_and(|d| d.dropping()) {
            phys::GRIND
        } else {
            phys
        };
        let up = s.normal;
        let surface = libm::acosf(up.y.clamp(-1.0, 1.0)) * (180.0 / crate::math::PI);
        let along = libm::asinf(s.fwd.y.clamp(-1.0, 1.0)) * (180.0 / crate::math::PI);
        let mut w = World {
            phys,
            speed: s.vel.length(),
            forward_speed: s.speed,
            speed_and_slope: speed_and_slope(s, up.y),
            mirrored: s.mirrored(),
            frame_turned: s.frame_turned,
            fakie: s.fakie,
            goofy: (s.stance == Stance::Goofy) != s.switch,
            switch: s.switch,
            time_since_input: idle,
            time_to_land,
            apex: matches!(s.mode, Mode::Air { .. }) && s.vel.y <= 0.0,
            surface_slope: surface,
            skate_slope: if s.speed < 0.0 { -along } else { along },
            push_brake_disabled: surface > 45.0,
            com_vel: [s.slip, s.vel.y, s.speed],
            wants_runout: matches!(s.mode, Mode::Runout { .. }),
            wants_wipeout: matches!(s.mode, Mode::Bailed { .. }),
            wants_manual_exit: s.manual_exit,
            // The speed wobble's amplitude, what Skate 3's animation turn conditioner reads.
            wobble: s.wobble.amplitude,
            // The deck against the body (the game puts the imported rider's posed board here instead).
            deck_yaw_pitch: {
                let a = s.deck_adjust;
                super::host::deck_angles(crate::math::v3(
                    libm::sinf(a.yaw) * libm::cosf(a.pitch),
                    libm::sinf(a.pitch),
                    libm::cosf(a.yaw) * libm::cosf(a.pitch),
                ))
            },
            grind: s.grind.as_ref().map_or(super::NONE, |g| {
                let name = g.graph_name();
                GRAPH_NAMES
                    .iter()
                    .position(|n| *n == name)
                    .map_or(super::NONE, |k| self.grinds[k])
            }),
            // A blunt (Skate 3's backslash family) and a darkslide's way out (SK8 grind/contact.rs:
            // on a ledge, 2 when the board's nose points to the high side, else 1; elsewhere by which
            // side of the rail the board is on).
            blunting: s
                .grind
                .as_ref()
                .is_some_and(|g| g.family == Family::Backslash),
            // SK8 grind_control/facing.rs: the deck's velocity against its nose, seen from above,
            // flipped riding switch and for the mirrored rider.
            grind_backwards: s.grind.is_some()
                && (s.vel.cross(s.fwd).y > 0.0) != (s.switch != s.mirrored()),
            grind_out: s.grind.as_ref().map_or(0, |g| darkslide_out(s, g)),
            slide: s
                .slide
                .as_ref()
                .map_or(0, |p| if p.side > 0.0 { 2 } else { 1 }),
            slide_ending: s.slide.as_ref().is_some_and(|p| p.releasing),
            // Only the trucks or the deck (a grind), not the wheels: SK8 TrucksOrDeckInContact reads the
            // truck and deck contacts. Counting the wheels (or nothing) wiped grabs out in the air.
            trucks_or_deck: s.mode == Mode::Grind,
            // Skate 3's processed turn (and its lean) are positive turning left, the way the board
            // yaws (`s.steer` is positive to the right): so the rider leans into the turn.
            turn: -s.steer,
            // The lean into the turn: the angle that balances the turn's centripetal pull (signed
            // like the turn, rolling either way).
            lean: libm::atanf(s.yaw_rate * s.speed / s.tuning.gravity.max(0.1)),
            ground_speed: s.ground_velocity().length(),
            lateral_up: up.cross(s.fwd).y,
            body_spin: s.spin_rate,
            vel_y: s.vel.y,
            vel_up: s.vel.dot(up),
            // SK8 PhysOutAnimation's crouching feedback: the pumping (its potential), the board's
            // absorption, the ground's compression and the least crouch.
            pump_potential: s.pump.pumping,
            crouch_force: s.pump.absorption,
            crouch_ground_force: s.pump.ground_absorption,
            crouch_min: s.pump.min_crouch,
            crouch_deck: s.pump.deck_absorption,
            // In the air, the rider's body over the board as the physics follows it (what SK8's air
            // idle reads at take-off: tucked, its legs stay in); riding, the clips' own.
            com_height: s.body_com.filter(|_| s.is_airborne()),
            // The landing classifier's signed magnitude (SK8 SetLandingData's spin).
            landing_spin: s.landing.map_or(0.0, |l| l.quality.magnitude),
            // Skate 3's landing classifier (0 straight, 1 sketchy, 2 spin, 3 slightly off).
            landing_type: s.landing.map_or(0, |l| l.quality.kind),
            body_flipping: s.body_flip.is_some(),
            pump_acc: s.pump.dv,
            slide_dir: {
                let v = s.vel.normalize_or(Vec3::ZERO);
                if v == Vec3::ZERO { 1.0 } else { v.dot(s.fwd) }
            },
            ground_acc: {
                // SK8 ground_acceleration.rs: the deck's up taken out, then the ground's, in the
                // board's frame.
                let a = s.accel - s.normal * s.accel.dot(s.normal);
                let a = a - s.contact_n * a.dot(s.contact_n);
                let side = s.contact_n.cross(s.fwd).normalize_or(Vec3::ZERO);
                [a.dot(side), a.dot(s.fwd)]
            },
            hips: match s.mode {
                // flatground's bail tips the body over sideways (as drawn) and keeps its heading
                // against the travel.
                Mode::Bailed { time } | Mode::Runout { time } => {
                    let lean = -(time * 5.0).min(1.0) * crate::math::PI * 0.48;
                    let v = s.vel.horizontal();
                    let twist = if v.length() > 0.3 {
                        let f = s.fwd.horizontal();
                        libm::atan2f(f.cross(v).y, f.dot(v))
                    } else {
                        0.0
                    };
                    [lean, twist]
                }
                _ => [0.0; 2],
            },
            toggling_board: s.board_toggle.is_some() || s.drop_in.is_some(),
            // The board off the feet (`skater::possession`).
            holding_board: s.possession.held(),
            deck_free: s.possession.free(),
            board_returning: s.possession.hold == crate::skater::possession::Hold::Returning,
            board_angle: s.possession.angle(s.rider_frame()),
            landing_on_board: s.landing_on_deck(),
            // The foot plant (`skater::plant`).
            foot_planting: s.air_plant.is_some(),
            footplant_length: s.air_plant.map_or(-1.0, |p| p.length()),
            footplant_contact: s.footplant.published_contact(),
            pose_com: s.body_com,
            // Dropping in (`skater::dropin`): on foot, or rolling slowly onto the coping on a tip.
            dropping_in: s.drop_in.is_some_and(|d| d.dropping())
                || s.grind.as_ref().is_some_and(|g| {
                    g.entry == crate::skater::Entry::DropIn && g.family == Family::Tipslide
                }),
            drop_in_flat: s.drop_in.is_some_and(|d| !d.edge.transition),
            // The hand plant (`skater::handplant`).
            handplant: s.handplant_world(),
            ..World::default()
        };
        // Coming down onto the free board (the caveman): `CanLandOnBoard` while the feet will meet its
        // deck, and whether it lies the wrong way round.
        if s.landing_on_free_board() {
            let (reach, turning) = s.free_board_landing(time_to_land.unwrap_or(0.0));
            w.can_land = reach;
            w.landing_turning = turning;
        }
        if s.mode == Mode::Walk {
            walking(&mut w, s);
        }
        w
    }

    /// One tick: pad -> action-graph intents, both graphs, animation. `clock`: a microsecond clock.
    pub fn tick(
        &mut self,
        db: &MotionDb,
        pad: &Pad,
        world: World,
        gates: Gates,
        dt: f32,
        clock: Option<fn() -> u32>,
    ) {
        let t0 = clock.map_or(0, |f| f());
        let r = &mut self.runner;
        self.input
            .produce(&r.pack, pad, gates, dt, &mut r.s.ag, &mut self.recognised);
        // Back on the board after a wipeout or runout (flatground's physics decides when): start the
        // graphs over, as Skate 3 does after a teleport.
        // (Not while Y is getting off the board: the dismount clips play on the rolling board.)
        let off = matches!(world.phys, phys::GROUND | phys::AIR)
            && !world.toggling_board
            && matches!(r.mg_state_root(), Some("WipeOut" | "Dismount" | "Default"));
        // Riding while the motion graph is still off the board (its clips holding the board): it never
        // gets back on by itself, so it is started over on the board once that has lasted a while.
        let riding_off = matches!(world.phys, phys::GROUND | phys::AIR)
            && !world.toggling_board
            && self.last_path.starts_with("OffBoard");
        self.off_board_ticks = if riding_off {
            self.off_board_ticks + 1
        } else {
            0
        };
        if off || self.off_board_ticks > OFF_BOARD_RESYNC {
            r.reset(Some(db));
            self.off_board_ticks = 0;
        }
        r.s.world = world;
        r.update(Some(db), dt);
        self.cost_us = clock.map_or(0, |f| f()).wrapping_sub(t0);
    }

    /// What the graphs decided for the physics this tick (after [`Driver::tick`]).
    pub fn control(&mut self, db: &MotionDb, t: &Tuning) -> GraphControl {
        let r = &self.runner;
        let path = r.motion_state();
        let has = |n: Option<NameId>| n.is_some_and(|n| r.attribute(db, n).is_some());
        let entered = |part: &str| path.contains(part) && !self.last_path.contains(part);
        let push_contact = has(self.clips.push_contact) || path.contains("/PushContact/");
        // From the push's first contact to its end (SK8's push contact events, down and back up).
        let pushing = push_contact
            || ["/PushCycle/", "/PushEnd/"]
                .iter()
                .any(|p| path.contains(p));
        let mut c = GraphControl {
            push_contact,
            pushing,
            push_target: push_contact.then_some(r.s.push_speed),
            // `BRAKE_CONTACT` is a value: 0 while the foot comes down (the brake's `INTO` clip), 1 once it
            // drags (`CYC`). Skate 3 brakes only on a contact of strength 1 (SK8 contact_events.rs).
            brake_contact: self
                .clips
                .brake_contact
                .and_then(|n| r.attribute(db, n))
                .is_some_and(|a| a.value() >= 0.5),
            kickturn: r.s.kickturn,
            // Off the board: the clips walking the rider (`OB_TRAJ`), getting off or on.
            ob_traj: has(self.clips.ob_traj),
            // ToggleBoard's throw clips on their channel: a throw's `OB_THROWBOARD`, a drop's
            // `OB_RELEASEBOARD` (or, without the clips, at once).
            release_board: {
                let on = |n: Option<NameId>| {
                    n.is_some_and(|n| {
                        r.s.player
                            .channel_attribute(db, super::host::CHANNEL_RETRIEVE, n)
                            .is_some()
                    })
                };
                if on(self.clips.throw) {
                    Some(true)
                } else if on(self.clips.release) {
                    Some(false)
                } else {
                    r.s.release_now
                }
            },
            retrieve_board: r.s.retrieve_pulse,
            land_on_board: r.packet_named("DoLandOnBoard").is_some(),
            ob_mounting: r.packet_named("OB_Mounting").is_some(),
            land_backwards: r.s.land_backwards,
            dismounting: r.mg_state_root() == Some("Dismount"),
            mounting: r.mg_state_root() == Some("Mount"),
            // The action graph publishing the stick and the motion graph in its board-adjust states
            // (until then the take-off's clip still poses the deck).
            board_adjust: r.board_adjusting() && path.contains("/BoardAdjusts/"),
            ..GraphControl::default()
        };
        if path.contains("/Manual/") {
            let lean = r
                .pack
                .find("Manual")
                .and_then(|m| r.s.mg.get(m))
                .map_or(0.0, f32::abs);
            c.manual = Some((
                path.contains("NoseManual"),
                lean.min(1.0),
                path.contains("Manual/Brake") || path.contains("Manual/Out"),
            ));
        }
        // Turning round from fakie to switch (`Switch`, `PushFromFakie`: both play `B_SWITCH`) happens
        // when its clip crosses the stance events, as in Skate 3; a library without them turns round
        // on entering those states.
        c.turn_round = if self.clips.mirrored.is_some() || self.clips.switch.is_some() {
            has(self.clips.mirrored) || has(self.clips.switch)
        } else {
            entered("/Switch/") || entered("/PushFromFakie/")
        };
        if entered("/Revert/") {
            c.revert = Some(path.contains("/Revert/Fs"));
        }
        // The takeoff clip's jump: pop on its first tick, as the trick the graph scores.
        // (On the ground or a rail: a takeoff clip still carrying its jump onto a rail is not a new pop.)
        let attr = has(self.clips.jump);
        let jump = attr && matches!(r.s.world.phys, phys::GROUND | phys::GRIND);
        if jump && !self.jumping {
            let name = r.str(r.s.trick);
            let name = name.strip_prefix("U_").unwrap_or(name);
            let flick = Flick::from_gesture(name).unwrap_or(if path.contains("Nollie") {
                Flick::NOLLIE
            } else {
                Flick::OLLIE
            });
            let gesture = r.s.mg.get(r.names.gesture_speed).unwrap_or(0.0);
            let height = self
                .clips
                .trick_height
                .and_then(|n| r.s.player.params.iter().find(|p| p.name == n))
                .map_or(gesture, |p| p.value);
            let over = r.packet(r.names.jump_override).unwrap_or(0.0);
            c.pop = Some((flick, jump_strength(height, over, t)));
        }
        self.jumping = attr;
        // Foot plants on the ground: the clips' board in hand and their jump off the planted foot (its
        // first tick), as the plant the graph scores.
        if r.s.world.phys == phys::GROUND {
            c.anim_skateboard = has(self.clips.anim_skateboard);
            let foot_jump = has(self.clips.foot_jump);
            c.foot_jump = foot_jump && !self.foot_jumping;
            self.foot_jumping = foot_jump;
        } else {
            self.foot_jumping = false;
        }
        if path.contains("/PushTrick/") {
            c.plant = crate::skater::plant::PlantKind::from_trick(r.str(r.s.trick));
        }
        // Hand plants: the world grab the motion graph attaches (Skate 3's RB: the physics looks for a
        // coping while it is held), its committed hand plant (`AnimCommittedHandPlant`), and the plant's
        // scored name (its one-foot and no-foot states' ScoringTrick, else ScoringHandPlants').
        c.grab_world = r.packet_named("GrabWorld").is_some();
        c.handplant_committed = r.packet_named("AnimCommittedHandPlant").is_some();
        if path.contains("HandPlantAir/") {
            let trick = r.str(r.s.trick);
            c.handplant_name = if trick.get(3..).is_some_and(|t| t.starts_with("handplant")) {
                Some(trick)
            } else if r.s.handplant_score != super::NONE {
                Some(r.pack.str(r.s.handplant_score))
            } else {
                None
            };
        }
        // Dropping in on foot: the motion graph standing at the edge (`IntoDropIn/Cycle`).
        c.drop_in_stand = path.contains("IntoDropIn/Cycle");
        c.drop_in_grinding = path.contains("Grinding/DroppingIn");
        // In the air: a one-foot grab runs the foot plant manager, its foot plant states reach for it.
        c.one_foot_air = r.packet_named("OneFootAir").is_some();
        c.foot_planting = r.packet_named("FootPlanting").is_some();
        // In the air: the trick the clips animate, whether the board is caught yet, a dark catch.
        // The danger zone's end (seconds into the clip; untimed: the whole clip): landing within the
        // last moment of it (`DANGER_GRACE`) still counts as caught.
        c.danger = self
            .clips
            .danger
            .and_then(|n| r.attribute(db, n))
            .is_some_and(|a| {
                let end = if a.end >= 0.0 {
                    a.end
                } else {
                    r.s.player.length()
                };
                end - r.s.player.time() > DANGER_GRACE
            });
        c.dark = r.s.dark;
        // The wind-up states' `IsAnticipating` (Skate 3's anticipating flag, which its camera reads as
        // `IsPreparingToJump`).
        c.anticipating = r.flag(super::ops::Flag::Anticipating);
        // A body flip the motion graph hands the physics (`AttachIntent FrontFlip` / `BackFlip`) and
        // the grab it reads with it.
        c.body_flip = if r.packet_named("FrontFlip").is_some_and(|v| v != 0.0) {
            Some(true)
        } else if r.packet_named("BackFlip").is_some_and(|v| v != 0.0) {
            Some(false)
        } else {
            None
        };
        c.grabbing = ["Grabbing", "WantsLeftAirGrab", "WantsRightAirGrab"]
            .iter()
            .any(|n| r.packet_named(n).is_some());
        // The graph went to its wipeout state on its own (the physics' own wipeouts take it there too,
        // once the physics is already off the board).
        c.wipeout = path.starts_with("WipeOut")
            && !self.last_path.starts_with("WipeOut")
            && matches!(r.s.world.phys, phys::GROUND | phys::AIR);
        if r.s.grab_score != super::NONE {
            c.grab_name = Some(r.pack.str(r.s.grab_score));
        }
        // Grinding: the grind grab the motion graph's `<grind>_<grab>` states score (`ScoringTrick`
        // `FS_5_O_FSGRAB` in their grab cycle), and the free end they hold up.
        if r.s.world.phys == phys::GRIND {
            c.grind_grab = crate::skater::balance::grab_of(r.str(r.s.trick));
            c.grind_grab_min_height = r.packet_named("PhysGrindGrabMinHeight").unwrap_or(0.0);
        }
        let tx = r.s.filtered.get(r.names.tweak_x).unwrap_or(0.0);
        let ty = r.s.filtered.get(r.names.tweak_y).unwrap_or(0.0);
        c.tweak = libm::sqrtf(tx * tx + ty * ty);
        c.body_spin = r.packet(r.names.phys_body_spin).unwrap_or(0.0);
        if r.s.world.phys == phys::AIR && air_trick_state(&path) {
            let name = r.str(r.s.trick);
            let (flick, flips) = air_flick(name);
            let len = r.s.player.length();
            c.air_trick = Some(AirTrick {
                flick,
                flips,
                progress: if len > 0.0 {
                    (r.s.player.time() / len).clamp(0.0, 1.0)
                } else {
                    1.0
                },
                late: [
                    "/Lateflip/",
                    "/DarkLateflip",
                    "/DarkToLightflip",
                    "FingerFlip",
                ]
                .iter()
                .any(|k| path.contains(k)),
                id: r.s.trick,
            });
        }
        self.last_path = path;
        c
    }

    /// The motion graph's state when it changed since the last call (for the log).
    pub fn state_change(&mut self) -> Option<String> {
        let now = self.runner.motion_state();
        if now == self.shown {
            return None;
        }
        self.shown = now.clone();
        Some(now)
    }

    /// Trees played with a parameter no behaviour has set (tree, parameter), each parameter once, since
    /// the last call.
    pub fn new_unset(&mut self, db: &MotionDb) -> Vec<(&'static str, String)> {
        let r = &mut self.runner;
        let out =
            r.s.player
                .unset
                .iter()
                .map(|&(t, n)| (r.pack.str(t), String::from(db.name(n))))
                .collect();
        r.s.player.unset.clear();
        out
    }

    /// Unsupported operations reached for the first time since the last call.
    pub fn new_unsupported(&mut self) -> Vec<&'static str> {
        let r = &mut self.runner;
        let names = r.s.new_unsupported.iter().map(|&s| r.pack.str(s)).collect();
        r.s.new_unsupported.clear();
        names
    }
}

/// `PhysicsSpeedAndSlopeCompare`'s value (SK8 riding/speed_and_slope.rs, Skate 3's heading adjust
/// factor): the downhill turn torque's share at this speed and slope, from the turn torque curves
/// against speed (as a share of the heading adjust's top speed) and slope (as a share of a right
/// angle; nothing past 1.2). Kick turns need it under 0.1.
pub fn speed_and_slope(s: &Skater, normal_y: f32) -> f32 {
    let t = &s.tuning;
    let mut slope = libm::acosf(normal_y.clamp(-1.0, 1.0)) / (crate::math::PI / 2.0);
    if slope > 1.2 {
        slope = 0.0;
    }
    let top = t.heading_adjust_max_speed.max(0.01);
    let k = s.speed.abs().min(top) / top;
    t.turn_torque_vs_speed.eval(k) * t.turn_torque_vs_slope.eval(slope)
}

/// Seconds before the end of a clip's `DANGERZONE` (the board not caught yet) from which a landing
/// counts as caught. flatground's own number: Skate 3 doesn't bail on the zone itself but makes its
/// physical wipeout checks more sensitive there, which flatground's landing rules don't model.
pub const DANGER_GRACE: f32 = 0.1;

/// The motion graph's states that animate a trick in the air: a flicked trick after its takeoff, late
/// flips, finger flips, under flips and dark catches.
pub fn air_trick_state(path: &str) -> bool {
    (path.contains("/Trick/") && !path.contains("/Takeoff/"))
        || [
            "/Lateflip/",
            "/DarkLateflip",
            "/DarkToLightflip",
            "FingerFlip",
            "Underflip",
            "DarkCatch",
        ]
        .iter()
        .any(|k| path.contains(k))
}

/// What a trick the graph scores (by name) looks like on the board, and its flips (a held kickflip
/// scores `Kickflip2`, `Kickflip3` ...).
pub fn air_flick(name: &str) -> (Flick, u8) {
    use crate::gesture::Trick;
    let digits = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let flips = name[digits.len()..].parse::<u8>().unwrap_or(1).clamp(1, 4);
    let base = digits.strip_prefix("U_").unwrap_or(digits);
    if let Some(f) = Flick::from_gesture(base) {
        return (f, flips);
    }
    let l = base.to_ascii_lowercase();
    let late = l.strip_prefix("l_b_").or_else(|| l.strip_prefix("l_f_"));
    let l = late.unwrap_or(&l);
    let trick = if l.contains("varial") && l.contains("heel") {
        Trick::VarialHeelflip
    } else if l.contains("varial") {
        Trick::VarialKickflip
    } else if l.contains("heelflip") {
        Trick::Heelflip
    } else if l.contains("fs_shuv") || l.contains("fsshuv") {
        Trick::FsPopShuvit
    } else if l.contains("shuv") || l.contains("_to_") {
        Trick::PopShuvit
    } else if l.contains("flip") || l.contains("dark") {
        Trick::Kickflip
    } else {
        Trick::Ollie
    };
    (Flick::of(trick), flips)
}

/// SK8's jump finalisation (animation attribute finalization): a trick height at or under the low clamp
/// is the minimum hop, at or over the high clamp full height; a flick-strength override can only raise it.
pub fn jump_strength(height: f32, override_height: f32, t: &Tuning) -> f32 {
    let h = if height <= t.jump_clamp_low {
        0.0
    } else if height >= t.jump_clamp_high {
        1.0
    } else {
        height
    };
    h.max(override_height).clamp(0.0, 1.0)
}

/// A darkslide's way out for the graph's `GrindTrickOutTypeAllowed` (SK8 grind/contact.rs): 0 for every
/// other grind; on a ledge 2 when the board's nose points to the high side, else 1; on a rail 1 when the
/// board's middle is to its nose side of the rail, else 2.
fn darkslide_out(s: &Skater, g: &crate::skater::Grind) -> u8 {
    if g.family != Family::Darkslide {
        return 0;
    }
    if g.class.is_ledge() {
        return if g.high.dot(s.fwd) > 0.0 { 2 } else { 1 };
    }
    let across = (s.pos - g.point).horizontal();
    if across.dot(s.fwd) > 0.0 { 1 } else { 2 }
}

/// What the graphs read of the rider on foot (Skate 3's biped publication): the locomotion state, the
/// board in hand, the air, the slope along the facing, and the velocity in the facing's frame.
fn walking(w: &mut World, s: &Skater) {
    let b = &s.biped;
    let f = b.forward();
    let n = b.normal;
    let dir = (f - n * f.dot(n)).normalize_or(f);
    let slope = libm::asinf(dir.y.clamp(-1.0, 1.0)) * (180.0 / crate::math::PI);
    let side = Vec3::UP.cross(f);
    w.loco = b.loco as u8;
    w.cadence = b.phase;
    w.holding_board = s.possession.held();
    w.biped_air = b.air.is_some();
    // In the air with ground to come down on (flatground has no jumps into walls). Onto the board only
    // once the landing assist has taken the flight down onto the free board (`Mode::Air`, the caveman).
    w.can_biped_land = b.air.is_some();
    w.can_land = s.deck_assist && s.free_board_landing(b.air_remaining).0;
    w.biped_slope = super::ops::slope::biped(slope, false);
    w.speed = b.speed;
    w.forward_speed = b.vel.dot(f);
    w.com_vel = [b.vel.dot(side), b.vel.y, b.vel.dot(f)];
    w.apex = b.air.is_some() && b.vel.y <= 0.0;
    w.surface_slope = libm::acosf(n.y.clamp(-1.0, 1.0)) * (180.0 / crate::math::PI);
    w.skate_slope = slope;
    w.hips = [0.0; 2];
    // SK8's off-board edge distance (`DistToEdge`): along the facing to where the ground falls away.
    w.dist_to_edge = s.drop_in.map_or(s.edge.map_or(10.0, |e| e.dist), |d| {
        (crate::math::v3(d.edge.point.x, s.pos.y, d.edge.point.z) - s.pos).length()
    });
    if let Some(time) = b.air {
        w.air_remaining = b.air_remaining;
        w.air_duration = time + b.air_remaining;
        let t = b.air_travel;
        w.air_travel = [t.dot(side), t.y, t.dot(f)];
    }
}
