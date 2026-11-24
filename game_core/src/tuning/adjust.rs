//! Board adjust in the air (the C-stick lines the deck up for a grind or a landing).
//!
//! Skate 3 has no tuning attribute for it: the action graph's BoardAdjust behaviour hands the C-stick to
//! the motion graph (`RightBoardAdjustMag`/`Angle`), whose `BLEND_ADJUST_*` clips pose the deck against
//! the body, and the physical board follows the animated one (SK8 `action_board_adjust.rs`,
//! `publish_deck_angles`). With the player's imported motion pack flatground's physics reads the posed
//! deck itself. These built-ins stand in for the clips without it: what the posed board reached and how
//! fast, measured through Skate 3's own graphs and clips (an ollie at 6 m/s, the C-stick held fully one
//! way from the take-off; goofy, unmirrored). They are numbers, not clip data.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdjustTuning {
    /// The deck's yaw against the body at a full C-stick (degrees): to the right (`BoardAdjusts/Right`:
    /// the nose swung towards the heel side in the clips' goofy stance) and to the left (`Left`: towards
    /// the toes).
    pub yaw_right: f32,
    pub yaw_left: f32,
    /// The deck's pitch at a full C-stick (degrees): pushed up the nose goes down (`BoardAdjusts/Up`),
    /// pulled down it comes up (`Down`).
    pub pitch_up: f32,
    pub pitch_down: f32,
    /// Share of the way to the stick's pose the deck goes a tick, and at most how far (degrees a tick).
    pub blend: f32,
    pub max_step: f32,
}

impl AdjustTuning {
    /// Measured from Skate 3's clips (see the module): the nose went to (-0.982, 0.174) right and
    /// (0.991, 0.027) left across the body, -0.337 and +0.406 up for up and down; the clips' blend moved
    /// it at most 13 degrees a tick, about half the rest of the way a tick near the end.
    pub const BUILT_IN: AdjustTuning = AdjustTuning {
        yaw_right: 80.0,
        yaw_left: 88.4,
        pitch_up: 19.7,
        pitch_down: 23.9,
        blend: 0.5,
        max_step: 13.0,
    };
}
