//! The unit tests' camera shots, standing in for Skate 3's from the player's disc
//! (`rules/camera/shots`; test builds only). The tree has the shots the tests' shot choice
//! (`select::built_in`) asks for, named like the camera graph's. The low camera's numbers are
//! ours; the high camera's shots (`bl_high_chase`, `high_grind`, `high_manual`, `bl_high_air`) and the
//! on-foot shot take Skate 3's distances, angles, smoothing and options (read off the disc; the blend
//! points of the high chase are ours).

use alloc::vec::Vec;

use super::shot::{
    NONE, OPT_DROP, OPT_FOLLOW, OPT_LOOK, OPT_MIRROR, OPT_PREVIOUS, REF_BOARD, REF_COM,
    REF_DAMPED_COM, REF_GRIND, REFS, Shot, ShotDef, ShotSet, name_hash,
};
use super::tracker::RAD;

/// A leaf: distance, elevation, heading, framing yaw, roll, pitch (degrees), look-at height.
#[derive(Clone, Copy)]
struct Leaf {
    distance: f32,
    elevation: f32,
    heading: f32,
    yaw: f32,
    roll: f32,
    pitch: f32,
    offset: f32,
}

const fn leaf(
    distance: f32,
    elevation: f32,
    heading: f32,
    yaw: f32,
    roll: f32,
    pitch: f32,
    offset: f32,
) -> Leaf {
    Leaf {
        distance,
        elevation,
        heading,
        yaw,
        roll,
        pitch,
        offset,
    }
}

struct Builder {
    defs: Vec<ShotDef>,
}

impl Builder {
    fn shot(
        &mut self,
        name: &str,
        l: Leaf,
        smoothing: [f32; 4],
        options: u8,
        transition: f32,
    ) -> u16 {
        let mut weights = [0.0; REFS];
        weights[REF_BOARD] = 1.0;
        self.push(
            name,
            Shot {
                distance: l.distance,
                lens: 12.0,
                smoothing,
                weights,
                board_offset: l.offset,
                heading: l.heading * RAD,
                elevation: l.elevation * RAD,
                framing: [l.roll * RAD, l.yaw * RAD, l.pitch * RAD],
                options,
                anchor: 0,
                compass: 0,
                collision: 0,
                arm_heading: l.heading * RAD,
                arm_elevation: l.elevation * RAD,
            },
            transition,
        )
    }

    fn push(&mut self, name: &str, shot: Shot, transition: f32) -> u16 {
        self.defs.push(ShotDef {
            key: name_hash(name),
            blend: false,
            shot,
            transition_time: transition,
            transition_units: 0,
            children: [NONE; 3],
            points: [0.0; 3],
            blend_value: 0.0,
            blend_type: 0,
            blend_smoothing: 0.0,
        });
        (self.defs.len() - 1) as u16
    }

    /// A blend of `children` by `kind` at `points`.
    fn blend(
        &mut self,
        name: &str,
        kind: u8,
        points: &[f32],
        children: &[u16],
        smoothing: f32,
        transition: f32,
    ) -> u16 {
        let i = self.push(name, Shot::START, transition) as usize;
        let d = &mut self.defs[i];
        d.blend = true;
        d.blend_type = kind;
        d.blend_smoothing = smoothing;
        for (k, (&p, &c)) in points.iter().zip(children).enumerate() {
            d.points[k] = p;
            d.children[k] = c;
        }
        d.shot.weights = [0.0; REFS];
        d.shot.weights[super::shot::REF_ANCHOR] = 1.0;
        i as u16
    }

    fn get(&mut self, i: u16) -> &mut Shot {
        &mut self.defs[i as usize].shot
    }
}

/// Speed (m/s, along the board), steering (+ left), predicted air time (s).
const SPEED: u8 = 1;
const STEER: u8 = 13;
const AIR_TIME: u8 = 16;

impl ShotSet {
    pub fn built_in() -> ShotSet {
        let mut b = Builder { defs: Vec::new() };
        let ride = OPT_FOLLOW | OPT_MIRROR | OPT_DROP;
        let calm = [0.9, 0.8, 0.9, 0.9];

        // Riding: a close shot off the rider's back side, swinging and tilting into turns; further
        // back and more central at speed.
        let slow_r = b.shot(
            "chase_slow_right",
            leaf(1.9, 8.0, 178.0, -20.0, -2.0, -5.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let slow = b.shot(
            "chase_slow",
            leaf(1.9, 8.0, -162.0, 13.0, 0.0, -5.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let slow_l = b.shot(
            "chase_slow_left",
            leaf(1.9, 8.0, 178.0, 20.0, 2.0, -5.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let fast_r = b.shot(
            "chase_fast_right",
            leaf(2.4, 6.0, 165.0, -18.0, 5.0, -3.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let fast = b.shot(
            "chase_fast",
            leaf(2.4, 6.0, 175.0, 3.0, 0.0, -3.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let fast_l = b.shot(
            "chase_fast_left",
            leaf(2.4, 6.0, -165.0, 18.0, -5.0, -3.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let bl_slow = b.blend(
            "bl_chase_slow",
            STEER,
            &[-0.75, 0.0, 0.75],
            &[slow_r, slow, slow_l],
            0.85,
            1.2,
        );
        let bl_fast = b.blend(
            "bl_chase_fast",
            STEER,
            &[-0.75, 0.0, 0.75],
            &[fast_r, fast, fast_l],
            0.88,
            1.2,
        );
        b.blend(
            "bl_chase",
            SPEED,
            &[9.0, 18.0],
            &[bl_slow, bl_fast],
            0.9,
            1.2,
        );

        // Turning hard: the frame swings further into the turn.
        let turn_r = b.shot(
            "turning_right",
            leaf(1.9, 9.0, 175.0, -26.0, -3.0, -5.0, 0.85),
            calm,
            ride,
            0.5,
        );
        let turn_l = b.shot(
            "turning_left",
            leaf(1.9, 9.0, -175.0, 26.0, 3.0, -5.0, 0.85),
            calm,
            ride,
            0.5,
        );
        b.blend(
            "bl_turning",
            STEER,
            &[-0.7, 0.0, 0.7],
            &[turn_r, slow, turn_l],
            0.85,
            0.8,
        );

        // Manuals: low and close when slow, further and higher when fast.
        let close = b.shot(
            "manual_close",
            leaf(1.2, -3.0, -158.0, 14.0, 0.0, -7.0, 0.5),
            calm,
            ride,
            0.5,
        );
        let far = b.shot(
            "manual_far",
            leaf(2.2, 11.0, -160.0, 6.0, 0.0, -3.0, 0.85),
            calm,
            ride,
            0.5,
        );
        b.blend("bl_manual", SPEED, &[4.0, 11.0], &[close, far], 0.9, 0.6);

        // Small airs: keep the shot, don't chase the board up (the look-at height ramps from the
        // take-off to the landing).
        let low = b.shot(
            "low_ollie",
            leaf(2.0, 0.0, 180.0, 0.0, 0.0, 0.0, 0.0),
            [0.9, 0.98, 0.85, 1.0],
            OPT_PREVIOUS | OPT_MIRROR,
            0.0,
        );
        let s = b.get(low);
        s.anchor = 5;
        s.weights = [0.0; REFS];
        s.weights[REF_COM] = 1.0;
        // Bigger airs: further back and higher the longer the flight.
        let air = [0.9, 0.95, 0.9, 0.65];
        let small = b.shot(
            "air_small",
            leaf(2.1, 6.0, -157.0, 0.0, 0.0, -12.0, 0.0),
            air,
            OPT_FOLLOW | OPT_MIRROR,
            0.5,
        );
        let big = b.shot(
            "air_big",
            leaf(3.8, 14.0, -157.0, 0.0, 0.0, -18.0, 0.0),
            air,
            OPT_FOLLOW | OPT_MIRROR,
            0.5,
        );
        for i in [small, big] {
            let s = b.get(i);
            s.anchor = 1;
            s.weights = [0.0; REFS];
            s.weights[REF_COM] = 1.0;
        }
        b.blend("bl_air", AIR_TIME, &[1.1, 3.2], &[small, big], 0.8, 0.5);

        // Grinds: close beside the rail, further at speed; ledges from the open side.
        let grind = [0.92, 0.8, 0.9, 0.9];
        let g_slow = b.shot(
            "grind_slow",
            leaf(1.6, 8.0, -158.0, 9.0, 1.0, -9.0, 0.6),
            grind,
            ride,
            0.5,
        );
        let g_fast = b.shot(
            "grind_fast",
            leaf(2.1, 7.0, -165.0, 5.0, 0.0, -5.0, 0.7),
            grind,
            ride,
            0.5,
        );
        b.blend("bl_grind", SPEED, &[8.0, 14.0], &[g_slow, g_fast], 0.9, 1.2);
        let ledge = OPT_FOLLOW | OPT_DROP;
        let l_slow = b.shot(
            "ledge_left_slow",
            leaf(1.7, 10.0, 152.0, -8.0, 0.0, -9.0, 0.6),
            grind,
            ledge,
            0.5,
        );
        let l_fast = b.shot(
            "ledge_left_fast",
            leaf(2.2, 9.0, 160.0, -5.0, 0.0, -6.0, 0.7),
            grind,
            ledge,
            0.5,
        );
        b.blend(
            "bl_grind_ledge_left",
            SPEED,
            &[5.0, 9.0],
            &[l_slow, l_fast],
            0.9,
            1.5,
        );
        let r_slow = b.shot(
            "ledge_right_slow",
            leaf(1.7, 10.0, -152.0, 8.0, 0.0, -9.0, 0.6),
            grind,
            ledge,
            0.5,
        );
        let r_fast = b.shot(
            "ledge_right_fast",
            leaf(2.2, 9.0, -160.0, 5.0, 0.0, -6.0, 0.7),
            grind,
            ledge,
            0.5,
        );
        b.blend(
            "bl_grind_ledge_right",
            SPEED,
            &[5.0, 9.0],
            &[r_slow, r_fast],
            0.9,
            1.5,
        );
        let exit = b.shot(
            "grind_exit",
            leaf(2.0, 0.0, 180.0, 0.0, 0.0, 0.0, 0.0),
            [0.9, 0.98, 0.85, 0.98],
            OPT_PREVIOUS | OPT_MIRROR | OPT_FOLLOW,
            0.8,
        );
        let s = b.get(exit);
        s.anchor = 5;
        s.weights = [0.0; REFS];
        s.weights[REF_COM] = 1.0;

        // Off the board: high and back, following the body.
        let wipe = b.shot(
            "wipeout",
            leaf(3.2, 32.0, 180.0, 0.0, 0.0, -6.0, 0.0),
            [0.8, 0.8, 0.8, 0.8],
            OPT_FOLLOW,
            0.2,
        );
        let s = b.get(wipe);
        s.anchor = 1;
        s.compass = 7;
        s.weights = [0.0; REFS];
        s.weights[REF_COM] = 0.9;
        s.weights[REF_DAMPED_COM] = 0.4;
        // On foot: Skate 3's `offboard_ground` (3 m back, 25 degrees up), the one ground shot whose
        // camera the look stick moves.
        let off = b.shot(
            "offboard_ground",
            leaf(3.0, 25.0, 180.0, 0.0, 0.0, -4.0, 0.0),
            [0.85, 0.8, 0.85, 0.85],
            OPT_FOLLOW | OPT_LOOK,
            0.5,
        );
        let s = b.get(off);
        s.anchor = 5;
        s.weights = [0.0; REFS];
        s.weights[REF_DAMPED_COM] = 1.0;

        // The high camera (Skate 3's `cameragraph_high` shots): further back and well above the rider.
        let high = OPT_FOLLOW | OPT_MIRROR;
        let chase = b.shot(
            "high_chase",
            leaf(3.0, 30.0, 180.0, 0.0, 0.0, 3.0, 1.0),
            [0.9; 4],
            high | OPT_DROP,
            0.0,
        );
        let chase_far = b.shot(
            "high_chase2",
            leaf(4.4, 26.6, 180.0, 0.0, 0.0, 5.0, 1.0),
            [0.7, 0.85, 0.85, 0.85],
            high | OPT_DROP,
            0.0,
        );
        b.blend(
            "bl_high_chase",
            SPEED,
            &[9.0, 18.0],
            &[chase, chase_far],
            0.95,
            2.0,
        );
        let hg = b.shot(
            "high_grind",
            leaf(2.9, 17.0, -160.0, 0.0, 0.0, 5.0, 1.0),
            [0.85; 4],
            high,
            0.0,
        );
        let s = b.get(hg);
        s.weights = [0.0; REFS];
        s.weights[REF_GRIND] = 1.0;
        b.shot(
            "high_manual",
            leaf(2.25, 19.0, -140.0, 0.0, 0.0, 0.0, 1.0),
            [0.85; 4],
            high,
            0.5,
        );
        let near = b.shot(
            "high_air",
            leaf(3.0, 10.0, -170.0, 0.0, 0.0, -10.0, 0.0),
            [0.95, 0.9, 0.9, 0.95],
            OPT_FOLLOW,
            0.0,
        );
        let s = b.get(near);
        s.anchor = 1;
        s.weights = [0.0; REFS];
        s.weights[REF_COM] = 1.0;
        let far_air = b.shot(
            "high_air_far",
            leaf(5.0, 0.0, -160.0, 0.0, 0.0, -25.0, 1.0),
            [0.95, 0.9, 0.9, 0.9],
            OPT_FOLLOW,
            0.0,
        );
        b.get(far_air).anchor = 1;
        b.blend(
            "bl_high_air",
            AIR_TIME,
            &[1.0, 3.5],
            &[near, far_air],
            0.7,
            0.0,
        );

        ShotSet {
            defs: b.defs,
            imported: false,
        }
    }
}
