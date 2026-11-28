//! The skater's physical state: Skate 3's (SK8 skate-core player/state.rs), moved between by Skate 3's
//! state selector (SK8 player/selector, TU3 `CalcSuggestedState` `0x82D8ADE8`) once a tick, as SK8's
//! skate-game physics/player_state/selection.rs and transition.rs do.
//!
//! The movement code doesn't change the state. It publishes what SK8's physics publishes for the
//! selector ([`Facts`]: the wheels touching, the wheel lines' gap, a jump, a wipeout request, a grind
//! contact, getting off or on the board), and the selector picks the state at the end of the tick, which
//! is where SK8's next tick selects (after its input phase, before its physics). Where the movement has
//! to know before it acts (touching down, locking onto a rail, stepping off) it asks the selector what
//! it would pick with that fact ([`Skater::selects`]), on a copy of its counters, so the selector alone
//! still decides.
//!
//! The processed-input bits and fields the selector reads (SK8 selector/input.rs names them by their
//! offsets) and what flatground publishes in them:
//!
//! | SK8 field | flatground |
//! |---|---|
//! | `wheel_contact_count_2556`, `SkateboardBody+869` | [`Facts::wheels`]: the rigid board's loaded wheels (in the air, those on a surface it can land on) |
//! | `SkateboardBody+856`, `+7692` | [`Facts::gap`], [`Facts::no_wheel_time`]: the wheel lines' nearest surface and the time with no wheel down |
//! | `field_2572` (state frames, reset by Processed2480 bit13) | [`Facts::jumped`]: a jump off the board's ground (a pop, a wallride's jump off) |
//! | Processed2468 bit8 (grinds flag_322) | [`Facts::left_grind`]: popped or come off the rail |
//! | Processed2468 bit10 (known trajectory) | the landing prediction is known (`Skater::plan`) |
//! | Processed2468 bit18 | [`Facts::wipeout`]: a bail or a runout |
//! | Processed2468 bits 16, 17 | [`Facts::deck`], [`Facts::trucks`]: the rigid board's deck or a truck against the world |
//! | `grind_candidate_1488`, `grind_type_1248` | the grind contact (`Skater::grind`) and its family |
//! | Processed2472 bit21, `field_2744` | a revert under way, and its direction |
//! | `field_2732` | a powerslide's side |
//! | `trajectory_collision_time_2772` | seconds to the known landing (negative: past it; -1 without one) |
//! | Processed2476 bit15 | on foot: staying off the board, or getting off it ([`Facts::dismount`]) |
//! | Processed2484 bit16 | on foot in the air |
//! | Processed2484 bit0 | getting on the board on the ground ([`Facts::mount`]) |
//! | `Skeleton+16420`, the board drive frame's nose y | the graphs' ForcePhysics mode (`Skater::force_mode`) and the posed deck's nose ([`Skater::board_animated`]) |

use super::Skater;
use crate::player::selector::conditions::{
    BoardBodyState, SkeletonAnimationState, TwoStageThresholds,
};
use crate::player::selector::{ProcessedStateInput, StateSelectionInput, StateSelector};
pub use crate::player::state::PhysicalStateId as State;

/// What the physics published this tick for the selector (cleared after each selection).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Facts {
    /// The board's wheels touching the ground (loaded); 4 at a touchdown from the air.
    pub wheels: u8,
    /// The wheel lines' nearest surface (m) and the time with no wheel touching (s), when off the
    /// ground (both 0 on it).
    pub gap: f32,
    pub no_wheel_time: f32,
    /// A jump this tick: a pop, a wallride's jump off, a mount in the air (the state frames start over).
    pub jumped: bool,
    /// Off the rail this tick (popped, or come off it).
    pub left_grind: bool,
    /// A wipeout (a bail or a runout) was asked for this tick.
    pub wipeout: bool,
    /// Getting off the board this tick.
    pub dismount: bool,
    /// Getting on the board this tick: `Some(true)` on the ground, `Some(false)` in the air.
    pub mount: Option<bool>,
    /// A grind contact found this tick, before it is taken (its family, `Family as u32`).
    pub grind: Option<u32>,
    /// The rigid board's deck, and either truck, against the world in this tick's step (SK8
    /// Processed2468 bits 16 and 17, which make the selector's `colliding` with the wheels).
    pub deck: bool,
    pub trucks: bool,
}

/// The selector's counters and the skater's state, kept together.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct States {
    pub current: State,
    /// Seconds in the current state (SK8 `state_timer_2664`; 0 on entering one).
    pub time: f32,
    /// SK8's state frames (`state_frames_1304`, published as `field_2572`): ticks since a jump reset them,
    /// 100 on a state change.
    pub frames: u32,
    pub selector: StateSelector,
    /// This tick's facts so far.
    pub facts: Facts,
    /// The wipeout is a runout (Skate 3's runout: off the board on the feet, back on it where it
    /// stops), not a bail.
    pub runout: bool,
}

impl Default for States {
    fn default() -> Self {
        States {
            current: State::PhysicsGround,
            time: 0.0,
            frames: 100,
            selector: StateSelector {
                post_grind_jump_counter: 11,
                ..StateSelector::default()
            },
            facts: Facts::default(),
            runout: false,
        }
    }
}

/// Skate 3's processed-input bits the selector reads (see the module's table).
mod bit {
    pub const LEFT_GRIND_2468: u32 = 1 << 8;
    pub const KNOWN_AIR_2468: u32 = 1 << 10;
    pub const WIPEOUT_2468: u32 = 1 << 18;
    pub const DECK_CONTACT_2468: u32 = 1 << 16;
    pub const TRUCK_CONTACT_2468: u32 = 1 << 17;
    pub const REVERT_2472: u32 = 1 << 21;
    pub const ON_FOOT_2476: u32 = 1 << 15;
    pub const BOARD_ON_GROUND_2484: u32 = 1;
    pub const ON_FOOT_AIR_2484: u32 = 1 << 16;
}

impl Skater {
    pub fn state(&self) -> State {
        self.states.current
    }

    /// The state the next tick's selection picks from what has been published so far (Skate 3's
    /// selector runs at the start of a tick, after the animation and before the physics: a pop's or a
    /// touchdown's tick is still the state it started in).
    pub fn next_state(&self) -> State {
        self.selects(|_| {})
    }

    /// Seconds in the current state.
    pub fn state_time(&self) -> f32 {
        self.states.time
    }

    /// Riding on the ground (Skate 3's ground states, category 100).
    pub fn rolling(&self) -> bool {
        self.states.current.category() == 100
    }

    /// Still riding the ground this tick: on it, and nothing the movement has published so far takes the
    /// selector off it (what was a check of the old mode mid-tick).
    pub(super) fn stays_rolling(&self) -> bool {
        self.rolling() && self.selects(|_| {}).category() == 100
    }

    /// The board in the air (Skate 3's air states, and the grind's in-between state, which flies).
    pub fn is_airborne(&self) -> bool {
        self.states.current.category() == 200 || self.states.current == State::Nonspecific
    }

    pub fn grinding(&self) -> bool {
        self.states.current.is_grind()
    }

    /// On foot off the board (Skate 3's off-board states, category 500).
    pub fn walking(&self) -> bool {
        self.states.current.category() == 500
    }

    /// Wiped out: a bail.
    pub fn bailed(&self) -> bool {
        self.states.current == State::WipeoutGround && !self.states.runout
    }

    /// Wiped out: a runout.
    pub fn running_out(&self) -> bool {
        self.states.current == State::WipeoutGround && self.states.runout
    }

    /// Off the board after a bail or a runout, and seconds since.
    pub fn off_board(&self) -> Option<f32> {
        (self.states.current == State::WipeoutGround).then_some(self.states.time)
    }

    /// Puts the skater in `state` at once, outside the selector: a respawn or a scripted placing (SK8's
    /// respawn and teleport set the state the same way), or the end of a runout.
    pub fn force_state(&mut self, state: State) {
        self.set_state(state);
        // What the last tick published was for the state it was in: the next selection starts from
        // the placing.
        self.states.facts = Facts::default();
        // Not just out of a grind: past the selector's wait before another (its post-grind jump
        // counter, which counts from 0 only after a grind jump).
        let c = &mut self.states.selector.post_grind_jump_counter;
        *c = (*c).max(11);
    }

    /// The selector's input from this tick's facts and the skater (SK8 player_state/selection.rs).
    fn selection_input(&self, f: &Facts) -> StateSelectionInput {
        let mut f2468 = 0;
        let mut f2472 = 0;
        let mut f2476 = 0;
        let mut f2484 = 0;
        if f.left_grind {
            f2468 |= bit::LEFT_GRIND_2468;
        }
        if self.plan.is_some() {
            f2468 |= bit::KNOWN_AIR_2468;
        }
        if f.wipeout {
            f2468 |= bit::WIPEOUT_2468;
        }
        if f.deck {
            f2468 |= bit::DECK_CONTACT_2468;
        }
        if f.trucks {
            f2468 |= bit::TRUCK_CONTACT_2468;
        }
        if self.revert.is_some() {
            f2472 |= bit::REVERT_2472;
        }
        let on_foot = self.walking() && f.mount.is_none();
        if on_foot || f.dismount {
            f2476 |= bit::ON_FOOT_2476;
        }
        // (Got on in the air last tick: the walker's flight is over, but it was the jump's.)
        let in_air = self.biped.air.is_some() || f.mount == Some(false);
        if self.walking() && in_air && f.mount != Some(true) {
            f2484 |= bit::ON_FOOT_AIR_2484;
        }
        if f.mount == Some(true) {
            f2484 |= bit::BOARD_ON_GROUND_2484;
        }
        let grind = f.grind.or(self.grind.as_ref().map(|g| g.family as u32));
        let trajectory = match &self.plan {
            Some(p) => p.remaining(self.air_time),
            None => -1.0,
        };
        let t = &self.tuning;
        let off_ground = TwoStageThresholds {
            field_856_primary: t.natural_air_max_dist,
            field_856_secondary: t.natural_air_min_dist,
            field_7692: t.natural_air_time,
        };
        StateSelectionInput {
            processed: ProcessedStateInput {
                grind_type_1248: grind.unwrap_or(6),
                grind_candidate_1488: grind.is_some(),
                field_1776: 0,
                flags_2468: f2468,
                flags_2472: f2472,
                flags_2476: f2476,
                flags_2480: 0,
                flags_2484: f2484,
                flags_2488: 0,
                category_2512: self.states.current.category(),
                wheel_contact_count_2556: i32::from(f.wheels),
                field_2572: if f.jumped {
                    1
                } else {
                    self.states.frames.saturating_add(1)
                },
                state_timer_2664: self.states.time,
                field_2732: self.slide.map_or(0.0, |s| s.side),
                field_2744: self.revert.map_or(0.0, |r| r.side),
                trajectory_collision_time_2772: trajectory,
                grind_investigation_flags_1516: 0,
            },
            skateboard_contact_count_869: f.wheels,
            board_body: BoardBodyState {
                field_856: f.gap,
                field_7692: f.no_wheel_time,
            },
            skeleton: self.skeleton_animation(),
            skitching_off_ground: off_ground,
            normal_off_ground: off_ground,
        }
    }

    /// What the selector reads of the skeleton (SK8 selection.rs): the ForcePhysics mode, the posed
    /// board's nose height in the world (the board drive frame's z axis y: the rider's frame turns the
    /// imported rider's posed deck; level without it) and `MaxDeckZAxisYForAnimatedDeck`.
    pub fn skeleton_animation(&self) -> SkeletonAnimationState {
        let nose_y = self.graph.pose_deck.map_or(0.0, |p| {
            let r = &self.reckoning;
            let turn = if self.frame_turned { -1.0 } else { 1.0 };
            let up = r.up;
            let right = up
                .cross(r.heading() * turn)
                .normalize_or(r.ground_right() * turn);
            let heading = right.cross(up).normalize_or(r.heading() * turn);
            (right * p.x + up * p.y + heading * p.z).y
        });
        SkeletonAnimationState {
            mode_16420: u32::from(self.force_mode),
            back_chain_lane_12656: nose_y,
            threshold_800: self.tuning.max_deck_z_axis_y_for_animated_deck,
        }
    }

    /// The clips move the board (TU3 `0x82BDC6C8`, SK8's GroundAnimation): forced so, or following
    /// the clips' data while they pitch it.
    pub fn board_animated(&self) -> bool {
        crate::player::selector::conditions::is_skateboard_animated(self.skeleton_animation())
    }

    /// The state the selector would pick now with this tick's facts changed by `with` (on a copy of
    /// its counters: nothing changes).
    pub(super) fn selects(&self, with: impl FnOnce(&mut Facts)) -> State {
        let mut f = self.states.facts;
        with(&mut f);
        let mut selector = self.states.selector;
        selector.calculate(self.states.current, &self.selection_input(&f))
    }

    /// The end of the tick's movement: the selector picks the state from what the physics published
    /// (SK8 selection.rs `advance`), and a new one is entered.
    pub(super) fn select_state(&mut self) {
        let f = self.states.facts;
        let input = self.selection_input(&f);
        self.states.frames = input.processed.field_2572;
        let current = self.states.current;
        let next = self.states.selector.calculate(current, &input);
        self.states.facts = Facts::default();
        if next != current {
            // SK8 82DB5FF4: a state change sets the state frames back to 100.
            self.states.frames = 100;
            self.set_state(next);
        }
    }

    /// Leaves the current state for `next` (SK8 transition.rs `set`): what each state's exit and
    /// entry do to the skater beyond the movement that led there.
    fn set_state(&mut self, next: State) {
        let current = self.states.current;
        self.states.current = next;
        self.states.time = 0.0;
        if next == current {
            return;
        }
        if current == State::WipeoutGround {
            self.states.runout = false;
        }
        if next.category() != 200 && next != State::Nonspecific {
            self.plant_clock = 0.0;
        }
        if !next.is_grind() && current.is_grind() {
            self.grind = None;
        }
    }

    /// The board leaves the ground this tick (a jump off it): publishes the jump.
    pub(super) fn publish_jump(&mut self) {
        self.states.facts.jumped = true;
        if self.grinding() {
            self.states.facts.left_grind = true;
        }
    }

    /// Publishes the board off the ground (no wheel down): the wheel lines' gap and the time since a
    /// wheel last touched.
    pub(super) fn publish_off_ground(&mut self, gap: f32, no_wheel_time: f32) {
        self.states.facts.wheels = 0;
        self.states.facts.gap = gap;
        self.states.facts.no_wheel_time = no_wheel_time;
    }

    /// Publishes the wheels on the ground.
    pub(super) fn publish_wheels(&mut self, wheels: u8) {
        self.states.facts.wheels = wheels;
        self.states.facts.gap = 0.0;
        self.states.facts.no_wheel_time = 0.0;
    }

    /// A touchdown from the air or the grind's in-between state: whether
    /// the selector takes the board onto the ground with it.
    pub(super) fn touchdown_lands(&self) -> bool {
        self.selects(|f| f.wheels = 4).category() == 100
    }

    /// Whether the selector takes a grind contact of `family` (`Family as u32`) found this tick.
    pub(super) fn grind_taken(&self, family: u32) -> bool {
        self.selects(|f| f.grind = Some(family)).is_grind()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flick::FlickOut;
    use crate::input::Pad;
    use crate::math::{Vec3, v3};
    use crate::park::Park;
    use crate::skater::{Event, GraphControl, Revert};
    use alloc::vec::Vec;

    const DT: f32 = 1.0 / 60.0;

    fn flat() -> Park {
        Park {
            shapes: Vec::new(),
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        }
    }

    fn rolling(speed: f32) -> (Park, Skater) {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.since_spawn = 10.0;
        s.speed = speed;
        s.graph = GraphControl::default();
        (park, s)
    }

    fn step(s: &mut Skater, park: &Park, n: usize) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..n {
            s.step(park, &Pad::default(), &FlickOut::default(), DT, &mut ev);
        }
        ev
    }

    /// SK8: the graphs' ForcePhysics sets the skeleton's board mode (`Skeleton+16420`), which the
    /// selector reads (TU3 0x82BDC6C8): forced animated, the board is the clips'; forced physical, it
    /// isn't; following the clips' data, only while the posed deck's nose rises or dips more than
    /// `MaxDeckZAxisYForAnimatedDeck` (0.02).
    #[test]
    fn the_graphs_force_mode_and_the_posed_nose_make_the_board_animated() {
        let (park, mut s) = rolling(4.0);
        step(&mut s, &park, 5);
        assert_eq!(s.tuning.max_deck_z_axis_y_for_animated_deck, 0.02);
        let level = Some(v3(0.0, 0.0, 1.0));
        let pitched = Some(v3(0.0, 0.07, 0.9975));
        for (mode, nose, animated) in [
            (2, level, true),
            (1, pitched, false),
            (0, level, false),
            (0, pitched, true),
            (0, Some(v3(0.0, -0.07, 0.9975)), true),
            (0, Some(v3(0.0, 0.015, 0.9999)), false),
            (0, None, false),
        ] {
            s.force_mode = mode;
            s.graph.pose_deck = nose;
            assert_eq!(s.board_animated(), animated, "mode {mode} nose {nose:?}");
        }
        // The physical state follows: GroundAnimation while animated, back to PhysicsGround after.
        s.force_mode = 2;
        step(&mut s, &park, 1);
        assert_eq!(s.state(), State::GroundAnimation);
        s.force_mode = 1;
        step(&mut s, &park, 1);
        assert_eq!(s.state(), State::PhysicsGround);
    }

    #[test]
    fn a_pop_is_a_jump_the_selector_takes_into_the_air_that_tick() {
        let (park, mut s) = rolling(3.0);
        step(&mut s, &park, 5);
        assert_eq!(s.state(), State::PhysicsGround);
        s.graph = GraphControl {
            pop: Some((crate::gesture::Flick::OLLIE, 1.0)),
            ..GraphControl::default()
        };
        step(&mut s, &park, 1);
        // SK8: the jump starts the state frames over, and `field_2572 == 1` is the ground states' way
        // into the air; the jump launched the landing prediction, which is known that tick (on flat
        // ground), so the air is KnownAir from the take-off as SK8's ollie (GroundAnimation, KnownAir).
        // The pop's tick is the ground state's: the next tick's selection (Skate 3's Logic, before
        // the physics) takes it into the air.
        assert_eq!(s.state().category(), 100);
        assert!(s.plan.is_some());
        s.graph = GraphControl::default();
        step(&mut s, &park, 1);
        assert_eq!(s.state(), State::KnownAir);
        assert_eq!(s.states.time, DT);
    }

    #[test]
    fn a_touchdown_lands_only_once_the_air_state_has_settled() {
        let (park, mut s) = rolling(3.0);
        s.pos = v3(0.0, 0.05, 0.0);
        s.vel = v3(0.0, -2.0, 3.0);
        s.force_state(State::PhysicsAir);
        let mut ticks = 0;
        while s.is_airborne() && ticks < 30 {
            step(&mut s, &park, 1);
            ticks += 1;
            assert!(s.pos.y >= -1e-3, "held on the ground, not through it");
        }
        assert!(s.rolling());
        // PhysicsAir takes a collision onto the ground only past 0.08 s (TU3's 0x3DA3D70A).
        assert!(ticks as f32 * DT > 0.08, "landed after {ticks} ticks");
    }

    #[test]
    fn a_revert_is_its_own_ground_state() {
        let (_, mut s) = rolling(3.0);
        s.revert = Some(Revert {
            side: 1.0,
            time: 0.0,
            turned: 0.0,
            rate: 0.0,
        });
        s.select_state();
        assert_eq!(s.state(), State::RevertGround);
        s.revert = None;
        s.select_state();
        assert_eq!(s.state(), State::PhysicsGround);
        assert!(s.states.selector.revert_exited_normally);
    }

    #[test]
    fn a_bail_is_the_wipeout_state_and_a_respawn_ends_it() {
        let (park, mut s) = rolling(3.0);
        s.bail("test", &mut Vec::new());
        s.select_state();
        assert_eq!(s.state(), State::WipeoutGround);
        assert!(s.bailed() && !s.running_out());
        s.respawn(&park);
        assert_eq!(s.state(), State::PhysicsGround);
    }

    #[test]
    fn getting_off_is_biped_ground_and_on_again_physics_ground() {
        let (_, mut s) = rolling(0.0);
        assert_eq!(s.selects(|f| f.dismount = true), State::BipedGround);
        s.states.facts.dismount = true;
        s.select_state();
        assert!(s.walking());
        // Staying on foot until getting on, then onto the board on the ground.
        s.select_state();
        assert_eq!(s.state(), State::BipedGround);
        s.states.facts.mount = Some(true);
        s.select_state();
        assert_eq!(s.state(), State::PhysicsGround);
    }
}
