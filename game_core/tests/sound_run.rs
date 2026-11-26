//! Sound end to end: scripted runs through the demo park with the sound layer listening, its commands
//! recorded (and mixed), and the simulation shown to be the same with and without it. Without imported
//! data a synthetic state graph pushes, pops and wipes out; the kicker run and the trick tour play
//! through Skate 3's own graphs with the player's imported data (skipped without it).

mod common;

use game_core::Game;
use game_core::autoplay::{KickerRun, TrickTour};
use game_core::gesture::{Flick, Trick};
use game_core::input::PadRaw;
use game_core::skater::{Event, GraphControl};
use game_core::sound::{Bank, Command, Cue, Mixer, Recorder, mix};

const TICK: f32 = 1.0 / 60.0;

/// A synthetic state graph's run at tick `t`: pushing for the first second, a kickflip at 1.5 s, another
/// at 4 s that wipes out in the air.
fn synthetic(game: &mut Game, t: u32) {
    let push = t < 60;
    game.skater.graph = GraphControl {
        push_contact: push,
        push_target: push.then_some(4.0),
        pushing: push,
        pop: (t == 90 || t == 240).then_some((Flick::of(Trick::Kickflip), 1.0)),
        wipeout: t == 255,
        ..GraphControl::default()
    };
}

fn idle() -> PadRaw {
    PadRaw {
        flags: game_core::input::flag::CONNECTED,
        ..Default::default()
    }
}

/// The cue a clip index belongs to (the first that has it).
fn cue_of(bank: &Bank, clip: u16) -> Option<Cue> {
    Cue::ALL
        .iter()
        .copied()
        .find(|c| bank.of(*c).contains(&clip))
}

/// Every Play of the run, with the frame it was in.
fn plays(rec: &Recorder, bank: &Bank) -> Vec<(usize, Cue, f32)> {
    let mut out = Vec::new();
    for (i, (cmds, _)) in rec.frames.iter().enumerate() {
        for c in cmds {
            if let Command::Play { clip, gain, .. } = *c
                && let Some(cue) = cue_of(bank, clip)
            {
                out.push((i, cue, gain));
            }
        }
    }
    out
}

#[test]
fn imported_the_kicker_run_sounds_like_a_kickflip() {
    let Some(mut game) = common::disc_game("imported_the_kicker_run_sounds_like_a_kickflip") else {
        return;
    };
    let mut script = KickerRun::new();
    let mut rec = Recorder::default();
    let mut events = Vec::new();
    for f in 0..(60 * 8) {
        let pad = script.next(&game);
        game.update(&pad, TICK);
        game.sound(&mut rec);
        events.extend(game.events.iter().map(|e| (f, e.clone())));
    }
    let bank = &game.sounds.clone();
    let p = plays(&rec, bank);
    let first = |cue: Cue| p.iter().find(|x| x.1 == cue).map(|x| x.0);
    // Pushing off: the foot, then the wheels rolling.
    assert!(first(Cue::Step).is_some(), "pushes: {p:?}");
    assert!(first(Cue::RollAsphaltSmooth).is_some(), "rolling: {p:?}");
    // The pop on the frame the pop happened, the flip's whoosh with it, the catch after, the landing
    // after that.
    let popped = events
        .iter()
        .find(|e| matches!(e.1, Event::Popped(_)))
        .map(|e| e.0)
        .expect("popped");
    assert_eq!(first(Cue::Pop), Some(popped));
    assert_eq!(first(Cue::Flip), Some(popped));
    let land = first(Cue::Land).expect("landed");
    assert!(land > popped);
    let catch = first(Cue::Catch).expect("caught");
    assert!(
        catch > popped && catch < land,
        "catch {catch}, pop {popped}, land {land}"
    );
    // The wheels spin in the air and are cut when they land.
    assert!(first(Cue::WheelSpin).is_some_and(|f| f >= popped && f < land));
    // The master volume reaches the platform.
    assert!(rec.frames.iter().all(|(_, m)| (*m - 0.8).abs() < 1e-6));
    game.set_volume(3);
    game.sound(&mut rec);
    assert!((rec.frames.last().unwrap().1 - 0.3).abs() < 1e-6);
    // Mixed, the run is not silent and doesn't clip much.
    let mut m = Mixer::new();
    let mut loud = 0usize;
    let mut clipped = 0usize;
    let mut out = vec![0i16; 1066];
    for (cmds, master) in &rec.frames {
        m.apply(bank, cmds);
        m.mix(bank, &mut out, *master);
        loud += out.iter().filter(|s| s.unsigned_abs() > 300).count();
        clipped += out.iter().filter(|s| s.unsigned_abs() >= 32767).count();
    }
    assert!(loud > 20_000, "{loud}");
    assert!(clipped < 100, "{clipped}");
}

#[test]
fn listening_does_not_change_the_simulation() {
    let run = |listen: bool| {
        let mut game = Game::new();
        let mut rec = Recorder::default();
        for t in 0..(60 * 10) {
            synthetic(&mut game, t);
            game.update(&idle(), TICK);
            if listen {
                game.sound(&mut rec);
            }
        }
        (
            game.skater.pos,
            game.skater.vel,
            game.score,
            game.ticks,
            rec,
        )
    };
    let (a, b) = (run(true), run(false));
    assert_eq!((a.0, a.1, a.2, a.3), (b.0, b.1, b.2, b.3));
    let bank = Bank::builtin();
    let p = plays(&a.4, &bank);
    let has = |cue: Cue| p.iter().any(|x| x.1 == cue);
    assert!(has(Cue::Step), "the push: {p:?}");
    assert!(has(Cue::Pop) && has(Cue::Land), "{p:?}");
    assert!(has(Cue::Bail), "{p:?}");
    let stops =
        a.4.frames
            .iter()
            .flat_map(|f| f.0.iter())
            .filter(|c| matches!(c, Command::Stop { .. }))
            .count();
    assert!(stops > 0);
    let mut m = Mixer::new();
    for (cmds, _) in a.4.frames.iter().take(120) {
        m.apply(&bank, cmds);
    }
    let pcm = mix::render(&mut m, &bank, 0.2, 1.0);
    assert!(pcm.iter().any(|s| *s != 0));
}

#[test]
fn imported_listening_to_the_trick_tour_does_not_change_the_simulation() {
    if common::data_dir().is_none() {
        eprintln!("imported_listening_to_the_trick_tour: no imported data (FG_DATA): skipped");
        return;
    }
    let run = |listen: bool| {
        let mut game = common::disc_game("imported_listening").unwrap();
        let mut script = TrickTour::new();
        let mut rec = Recorder::default();
        for _ in 0..(60 * 60) {
            let pad = script.next(&game);
            if let Some(p) = script.take_place() {
                game.place(&p);
            }
            game.update(&pad, TICK);
            if listen {
                game.sound(&mut rec);
            }
        }
        (
            game.skater.pos,
            game.skater.vel,
            game.score,
            game.ticks,
            rec,
        )
    };
    let (a, b) = (run(true), run(false));
    assert_eq!((a.0, a.1, a.2, a.3), (b.0, b.1, b.2, b.3));
    // The tour's tricks sound, and the loops stop.
    let bank = Bank::builtin();
    let p = plays(&a.4, &bank);
    let has = |cue: Cue| p.iter().any(|x| x.1 == cue);
    assert!(has(Cue::Pop) && has(Cue::Land), "{p:?}");
    let stops =
        a.4.frames
            .iter()
            .flat_map(|f| f.0.iter())
            .filter(|c| matches!(c, Command::Stop { .. }))
            .count();
    assert!(stops > 0);
    // Rendered: a second of the start is quiet only before the first push.
    let mut m = Mixer::new();
    for (cmds, _) in a.4.frames.iter().take(120) {
        m.apply(&bank, cmds);
    }
    let pcm = mix::render(&mut m, &bank, 0.2, 1.0);
    assert!(pcm.iter().any(|s| *s != 0));
}

#[test]
fn the_menu_silences_the_loops() {
    let mut game = Game::new();
    let mut rec = Recorder::default();
    for t in 0..90 {
        synthetic(&mut game, t);
        game.update(&idle(), TICK);
        game.sound(&mut rec);
    }
    assert!(game.director.rolling().is_some(), "rolling after the push");
    let start = game_core::input::PadRaw {
        flags: game_core::input::flag::CONNECTED,
        buttons: game_core::input::btn::START,
        ..Default::default()
    };
    game.update(&start, TICK);
    game.sound(&mut rec);
    assert!(game.menu.is_some());
    assert!(game.director.rolling().is_none());
    assert!(
        rec.frames
            .last()
            .unwrap()
            .0
            .iter()
            .any(|c| matches!(c, Command::Stop { .. }))
    );
}

/// The player's converted sounds (`FG_DATA`, else `dist/apps/flatground/data`): every cue comes from the
/// disc, every clip decodes to sound (not silence, not noise at full scale) and the pack fits its budget.
/// Without imported data the test passes and says so.
#[test]
fn imported_sounds_load_and_play() {
    let dir = std::env::var_os("FG_DATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../dist/apps/flatground/data")
        });
    let Ok(bytes) = std::fs::read(dir.join("sound.fgpak")) else {
        eprintln!("imported sounds: no sound.fgpak (FG_DATA): skipped");
        return;
    };
    let pack: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    assert!(
        pack.len() < 4 << 20,
        "{} bytes: over the 4 MiB budget",
        pack.len()
    );
    // The rest of the player's data too, when it is there (the scripted run plays through the graphs).
    let mut game = common::disc_game("imported_sounds_load_and_play").unwrap_or_default();
    let (cues, clips) = game.load_sounds(pack).expect("sound pack");
    assert_eq!(cues, Cue::COUNT, "every cue from the disc");
    assert!(clips >= Cue::COUNT);
    let bank = &game.sounds;
    assert_eq!(bank.imported(), Cue::COUNT);
    for cue in Cue::ALL {
        let n = bank.of(cue).len();
        if cue.is_roll() {
            assert_eq!(n, game_core::sound::ROLL_BANDS, "{cue:?}");
        }
        for &i in bank.of(cue) {
            let c = bank.clip(i).unwrap();
            assert_eq!(c.rate, game_core::sound::RATE);
            assert!(c.peak > 200, "{cue:?} clip {i} is silent");
            assert!(
                c.seconds() > 0.02 && c.seconds() < 4.0,
                "{cue:?} {}",
                c.seconds()
            );
            // Played alone, its mix reaches about its peak (the decoder and the encoder agree).
            let mut m = Mixer::new();
            m.apply(
                bank,
                &[Command::Play {
                    voice: 1,
                    clip: i,
                    gain: 1.0,
                    pan: -1.0,
                    pitch: 1.0,
                    fade: 0.0,
                }],
            );
            let out = mix::render(&mut m, bank, c.seconds().min(1.0), 1.0);
            let peak = out
                .iter()
                .step_by(2)
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap();
            assert!(
                (peak as f32) > c.peak as f32 * 0.7 && (peak as f32) < c.peak as f32 * 1.3 + 50.0,
                "{cue:?} clip {i}: mixed peak {peak}, clip peak {}",
                c.peak
            );
        }
    }
    // A scripted run with them (through Skate 3's graphs): the same sounds fire as with the placeholders.
    if game.graph.is_none() {
        return;
    }
    let mut script = KickerRun::new();
    let mut rec = Recorder::default();
    for _ in 0..(60 * 8) {
        let pad = script.next(&game);
        game.update(&pad, TICK);
        game.sound(&mut rec);
    }
    let p = plays(&rec, &game.sounds);
    for cue in [
        Cue::Pop,
        Cue::Land,
        Cue::Catch,
        Cue::Step,
        Cue::RollAsphaltSmooth,
    ] {
        assert!(p.iter().any(|x| x.1 == cue), "{cue:?}: {p:?}");
    }
}
