//! The drawn board never turns round under the rider after a landing (a player saw it on the Wii: the
//! game logged `board: turned 95 deg under the rider 7 frames after SKETCHY SWITCH NOLLIE VARIAL
//! HEELFLIP`, `97 deg ... after SKETCHY SWITCH BS 360 OLLIE`, and `180 deg ... 34 frames after MANUAL`).
//!
//! - Landing on a deck lined up with board adjust (the C-stick still over at touchdown, so the board
//!   lands sketchy, turned against the travel): the physical board is the adjusted deck, and Skate 3
//!   places its animation so the posed board lies on it every ground tick (SK8 `SkeletonRootFrames::
//!   update`, its heading alignment re-initialised on each Ground update). As the board-adjust clip
//!   blends out the rider turns round the board, never the board under him. Riding switch (a goofy
//!   rider who pushed out of fakie, the board's frame turned end for end) and riding normal.
//! - Out of a tail manual into a 180: the manual's score comes as it pops, and the board turns with the
//!   rider in the spin; landing fakie isn't the board turning under him.
//! - A 180 landed fakie and rolled tail first into a wall: the wall pushes the rider back nose first
//!   and his travel turns round, not the board. The game's watch measures the drawn board against the
//!   physical one, end for end (it measured against the travel and logged this as a 180-degree turn).
//!
//! Needs the player's imported data (`FG_DATA`, else `dist/apps/flatground/data`); without it the
//! tests pass and say so.

use std::path::PathBuf;

use game_core::Game;
use game_core::input::{PadRaw, btn, flag};
use game_core::math::{PI, Vec3, atan2f, v3, wrap_angle};
use game_core::skater::{Event, Mode};

const TICK: f32 = 1.0 / 60.0;

fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist/apps/flatground/data")
        });
    ["skater.fgpak", "motion.fgpak", "rules.fgpak"]
        .iter()
        .all(|f| dir.join(f).is_file())
        .then_some(dir)
}

fn leak(path: PathBuf) -> &'static [u8] {
    Box::leak(std::fs::read(path).expect("read pack").into_boxed_slice())
}

fn imported_game(test: &str) -> Option<Game> {
    let Some(dir) = data_dir() else {
        eprintln!("{test}: no imported data (FG_DATA): skipped");
        return None;
    };
    let mut g = Game::new();
    g.load_rules(&std::fs::read(dir.join("rules.fgpak")).unwrap())
        .expect("rules pack");
    g.load_skater(leak(dir.join("skater.fgpak")))
        .expect("skater pack");
    g.load_motion(leak(dir.join("motion.fgpak")))
        .expect("motion pack");
    assert!(g.graph.is_some(), "state graphs on");
    Some(g)
}

/// A pad: the main stick across, the C-stick, buttons.
fn pad(stick_x: i8, c: (i8, i8), buttons: u32) -> PadRaw {
    PadRaw {
        stick_x,
        cstick_x: c.0,
        cstick_y: c.1,
        flags: flag::CONNECTED,
        buttons,
        ..Default::default()
    }
}

fn yaw(v: Vec3) -> f32 {
    atan2f(v.x, v.z)
}

/// The drawn board's long axis in the world (the posed `SKATEBOARD_ROOT` placed where it is drawn).
fn drawn_board_yaw(g: &Game) -> f32 {
    let r = g.rider().expect("imported rider");
    let c = &r.character;
    let b = c.world()[c.bone("SKATEBOARD_ROOT").expect("board bone")];
    let x = g.imported_xform();
    yaw(x.point(v3(b[2][0], b[2][1], b[2][2])) - x.pos)
}

/// How far (degrees) two long axes are apart, either end.
fn axis_apart(a: f32, b: f32) -> f32 {
    let d = wrap_angle(a - b).abs().to_degrees();
    d.min(180.0 - d)
}

/// What the run saw: the landing's name, and over the 45 frames after it the drawn board's largest
/// step (degrees a frame), its largest gap from the physical board's long axis, and the game's
/// `board: ...` log lines.
#[derive(Debug, Default)]
struct Seen {
    landed: Option<String>,
    manual: bool,
    step: f32,
    off_deck: f32,
    logged: Vec<String>,
    /// Where the rider landed and his speed along the board then (m/s, negative rolling tail first).
    landed_at: (Vec3, f32),
    /// The travel turned round (a wall pushed him back) while the board stayed as it was.
    bounced: bool,
}

/// Rides the open lane along +X on the built-in park, feeding `script(frame)`, and watches the drawn
/// board after the first trick landing (a manual's "landing" doesn't count).
fn ride(g: &mut Game, fakie: bool, script: impl Fn(u32) -> PadRaw) -> Seen {
    if fakie {
        g.place_rider(v3(-18.0, 0.0, -15.0), -PI / 2.0, -6.0);
    } else {
        g.place_rider(v3(-18.0, 0.0, -15.0), PI / 2.0, 6.0);
    }
    watch(g, script)
}

/// Feeds `script(frame)` from where the rider is and watches the drawn board after the first trick
/// landing ([`ride`]).
fn watch(g: &mut Game, script: impl Fn(u32) -> PadRaw) -> Seen {
    let mut seen = Seen::default();
    let mut since: Option<u32> = None;
    let mut last = drawn_board_yaw(g);
    for i in 0..300u32 {
        g.update(&script(i), TICK);
        for e in &g.events {
            seen.manual |= matches!(e, Event::Landed { name, .. } if name == "MANUAL");
            if let Event::Landed { name, .. } = e
                && name != "MANUAL"
                && seen.landed.is_none()
            {
                seen.landed = Some(name.clone());
                since = Some(0);
            }
        }
        seen.logged
            .extend(g.log.drain(..).filter(|l| l.starts_with("board: ")));
        let now = drawn_board_yaw(g);
        if let Some(n) = &mut since {
            assert_eq!(g.skater.mode, Mode::Rolling, "rode away from the landing");
            seen.step = seen.step.max(wrap_angle(now - last).abs().to_degrees());
            seen.off_deck = seen.off_deck.max(axis_apart(now, yaw(g.skater.fwd)));
            if *n == 0 {
                seen.landed_at = (g.skater.pos, g.skater.speed);
            }
            seen.bounced |= g.skater.speed * seen.landed_at.1 < 0.0;
            *n += 1;
            if *n > 45 {
                break;
            }
        }
        last = now;
    }
    seen
}

/// Pushes out of fakie (the rider turns round to switch, the board's frame end for end) when `switch`,
/// winds up and ollies at frame 50..84, then holds the main stick over by `spin` for `spin_frames`
/// and the C-stick across by `adjust` from frame 95 to `adjust_to` (board adjust).
fn ollie(
    switch: bool,
    spin: i8,
    spin_frames: u32,
    adjust: i8,
    adjust_to: u32,
) -> impl Fn(u32) -> PadRaw {
    move |i| {
        let spinning = (84..84 + spin_frames).contains(&i);
        let adjusting = (95..adjust_to).contains(&i);
        match i {
            22..28 if switch => pad(0, (0, 0), btn::A),
            50..80 => pad(0, (0, -92), 0),
            80..84 => pad(0, (0, 92), 0),
            _ => pad(
                if spinning { spin } else { 0 },
                (if adjusting { adjust } else { 0 }, 0),
                0,
            ),
        }
    }
}

fn assert_board_stays(seen: &Seen, want: &str) {
    eprintln!("{want}: {seen:?}");
    let landed = seen.landed.as_deref().expect("landed");
    assert!(landed.contains(want), "{landed}");
    assert!(seen.logged.is_empty(), "{:?}", seen.logged);
    // The board lands as it was lined up and stays on the physical board, which grip straightens only
    // slowly; the rider turns round it.
    assert!(
        seen.off_deck < 5.0,
        "drawn board {:.0} deg off the physical one",
        seen.off_deck
    );
    assert!(
        seen.step < 12.0,
        "drawn board turned {:.0} deg in a frame",
        seen.step
    );
}

#[test]
fn a_switch_landing_on_an_adjusted_deck_turns_the_rider_not_the_board() {
    // The Wii's 32.448: SKETCHY SWITCH ..., the board 37 degrees off; it turned 95 degrees under him.
    let Some(mut g) = imported_game("switch adjusted landing") else {
        return;
    };
    let seen = ride(&mut g, true, ollie(true, 0, 0, 80, 300));
    assert!(g.skater.switch && g.skater.frame_turned, "rode switch");
    assert_board_stays(&seen, "SKETCHY SWITCH OLLIE");
    let mut g = imported_game("switch adjusted landing").unwrap();
    let seen = ride(&mut g, true, ollie(true, 0, 0, -80, 300));
    assert_board_stays(&seen, "SKETCHY SWITCH OLLIE");
}

#[test]
fn a_switch_spin_landed_on_an_adjusted_deck_turns_the_rider_not_the_board() {
    // The Wii's 35.368: SKETCHY SWITCH BS 360 OLLIE with the C-stick over; it turned 97 degrees.
    let Some(mut g) = imported_game("switch adjusted spin") else {
        return;
    };
    let seen = ride(&mut g, true, ollie(true, 100, 24, 80, 136));
    assert_board_stays(&seen, "SWITCH BS 180 OLLIE");
    let mut g = imported_game("switch adjusted spin").unwrap();
    let seen = ride(&mut g, true, ollie(true, -100, 24, 80, 300));
    assert_board_stays(&seen, "SWITCH FS 180 OLLIE");
}

#[test]
fn a_regular_landing_on_an_adjusted_deck_turns_the_rider_not_the_board() {
    let Some(mut g) = imported_game("adjusted landing") else {
        return;
    };
    let seen = ride(&mut g, false, ollie(false, 0, 0, -80, 300));
    assert!(!g.skater.switch && !g.skater.frame_turned);
    assert_board_stays(&seen, "SKETCHY OLLIE");
}

#[test]
fn a_180_out_of_a_manual_does_not_turn_the_board_under_the_rider() {
    // The Wii's 29.245: out of a tail manual into a FS 180, landed fakie; logged as the board turning
    // 180 degrees 34 frames after MANUAL.
    let Some(mut g) = imported_game("manual 180") else {
        return;
    };
    let seen = ride(&mut g, false, |i| match i {
        20..80 => pad(0, (0, -50), 0),
        80 => pad(0, (0, -92), 0),
        81..83 => pad(0, (50, 62), 0),
        83..108 => pad(100, (0, 0), 0),
        _ => pad(0, (0, 0), 0),
    });
    assert!(seen.manual, "the manual scored as it popped");
    assert!(g.skater.fakie, "landed fakie");
    assert_board_stays(&seen, "FS 180 OLLIE");
}

#[test]
fn a_180_landed_fakie_into_a_wall_does_not_turn_the_board_under_the_rider() {
    // The trick tour's BS 180 to fakie, with its kicker flicked at other points, landed by the kicker
    // and rolled tail first into its side: the wall pushed the rider back (`hit_wall`: the speed into
    // it goes, `physics_collision/TargetDisplacementVel` out) and he rolled on nose first. The board
    // never turned; the travel did, and the game logged "board: turned 180 deg ... after BS 180
    // OLLIE". Here a 180 lands fakie in the south lane just short of the park's south wall.
    for (spin, want) in [(100, "FS 180 OLLIE"), (-100, "BS 180 OLLIE")] {
        let Some(mut g) = imported_game("180 into a wall") else {
            return;
        };
        g.place_rider(v3(-5.0, 0.0, -4.0), PI, 6.0);
        let seen = watch(&mut g, ollie(false, spin, 24, 0, 0));
        assert!(seen.landed_at.1 < 0.0, "landed fakie: {seen:?}");
        assert!(seen.bounced, "the wall pushed him back: {seen:?}");
        assert!(!g.skater.fakie, "rolling nose first after the wall");
        assert_board_stays(&seen, want);
    }
}
