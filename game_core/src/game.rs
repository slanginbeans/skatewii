//! One session in the park: input -> flick reader -> skater -> camera, then drawing.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::camera::ChaseCam;
use crate::flick::FlickReader;
use crate::gfx::{Gfx, MeshBuilder, SCREEN_W, Vertex, rgb};
use crate::hud;
use crate::input::{Pad, PadRaw, flag};
use crate::math::{Mat3, PI, TAU, Xform, sinf, v3};
use crate::model::Models;
use crate::park::Park;
use crate::skater::{Event, Mode, Skater};

const SKY: u32 = rgb(150, 196, 232);
const POPUP_TIME: f32 = 1.8;
const HINT_TIME: f32 = 12.0;

struct Popup {
    title: String,
    detail: String,
    rgba: u32,
    time: f32,
}

pub struct Game {
    pub park: Park,
    park_mesh: Vec<Vertex>,
    models: Models,
    pub skater: Skater,
    flick: FlickReader,
    cam: ChaseCam,
    pub score: u32,
    popup: Option<Popup>,
    prev_held: u32,
    clock: f32,
    connected: bool,
    /// Events from the last `update` (cleared every frame). Handy for tests and the preview tool.
    pub events: Vec<Event>,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Game {
        let park = Park::demo();
        let skater = Skater::spawn(&park);
        Game {
            park_mesh: park.build_mesh(),
            models: Models::build(),
            cam: ChaseCam::new(&skater),
            park,
            skater,
            flick: FlickReader::new(),
            score: 0,
            popup: None,
            prev_held: 0,
            clock: 0.0,
            connected: true,
            events: Vec::new(),
        }
    }

    /// One fixed step (1/60 s on NTSC and PAL60, 1/50 s on PAL50).
    pub fn update(&mut self, raw: &PadRaw, dt: f32) {
        let pad = Pad::from_raw(raw, self.prev_held);
        self.prev_held = raw.buttons;
        self.connected = raw.flags & flag::CONNECTED != 0;
        self.clock += dt;
        self.events.clear();

        let flick = self.flick.update(pad.flick, dt);
        let rolling = self.skater.mode == Mode::Rolling;
        let flick = if rolling { flick } else { None };
        let loaded = rolling && self.flick.is_loaded();
        self.skater
            .step(&self.park, &pad, flick, loaded, dt, &mut self.events);
        self.cam.update(&self.skater, &self.park, dt);

        for e in &self.events {
            match e {
                Event::Landed { name, points } => {
                    self.score += points;
                    self.popup = Some(Popup {
                        title: name.clone(),
                        detail: format!("+{points}"),
                        rgba: rgb(255, 230, 90),
                        time: 0.0,
                    });
                }
                Event::Bailed => {
                    self.popup = Some(Popup {
                        title: "BAIL!".into(),
                        detail: String::new(),
                        rgba: rgb(255, 90, 70),
                        time: 0.0,
                    });
                }
                _ => {}
            }
        }
        if let Some(p) = &mut self.popup {
            p.time += dt;
            if p.time > POPUP_TIME {
                self.popup = None;
            }
        }
    }

    pub fn render(&self, gfx: &mut dyn Gfx) {
        gfx.begin_frame(&self.cam.camera(), SKY);
        gfx.draw_tris(&self.park_mesh, &Xform::IDENTITY.to_mat34());
        self.draw_skater(gfx);
        self.draw_hud(gfx);
        gfx.end_frame();
    }

    fn draw_skater(&self, gfx: &mut dyn Gfx) {
        let s = &self.skater;
        let stance = if s.fakie { PI } else { 0.0 };
        let base = Xform::new(
            Mat3::yaw_on_surface(s.heading + s.spin + stance, s.normal),
            s.pos,
        );

        // Board: lifts and turns under the feet while a trick plays.
        let (mut lift, mut board_rot) = (0.0, Mat3::IDENTITY);
        if let Some(t) = &s.trick {
            let p = t.progress();
            lift = 0.28 * sinf(PI * p);
            board_rot = Mat3::rot_y(t.def.shove_halves * PI * p)
                .mul(&Mat3::rot_z(t.def.flip_turns * TAU * p));
        }
        let pivot = 0.09;
        let board = base
            .then(&Xform::new(board_rot, v3(0.0, pivot + lift, 0.0)))
            .then(&Xform::new(Mat3::IDENTITY, v3(0.0, -pivot, 0.0)));
        gfx.draw_tris(&self.models.board, &board.to_mat34());

        // Rider: tucks with the board, falls over on a bail.
        let (rider_pos, rider_rot) = match s.mode {
            Mode::Bailed { time } => (
                v3(0.0, 0.0, 0.0),
                Mat3::rot_z(-(time * 5.0).min(1.0) * PI * 0.48),
            ),
            _ => (v3(0.0, lift * 0.7, 0.0), Mat3::IDENTITY),
        };
        let rider = base.then(&Xform::new(rider_rot, rider_pos));
        let tucked = s.crouch > 0.5 || (s.is_airborne() && lift > 0.05);
        let mesh = if tucked {
            &self.models.rider_crouched
        } else {
            &self.models.rider
        };
        gfx.draw_tris(mesh, &rider.to_mat34());
    }

    fn draw_hud(&self, gfx: &mut dyn Gfx) {
        let mut m = MeshBuilder::new();
        let white = rgb(255, 255, 255);
        hud::text(
            &mut m,
            24.0,
            22.0,
            4.0,
            &format!("SCORE {}", self.score),
            white,
        );
        let kmh = (self.skater.speed.abs() * 3.6) as u32;
        let speed = format!("{kmh} KM/H");
        hud::text(
            &mut m,
            SCREEN_W - 24.0 - hud::text_width(&speed, 4.0),
            22.0,
            4.0,
            &speed,
            white,
        );

        if let Some(p) = &self.popup {
            // Pop in with a slight scale-up.
            let px = 6.0 + 2.0 * (1.0 - (p.time * 8.0).min(1.0));
            hud::text_centered(&mut m, SCREEN_W / 2.0, 120.0, px, &p.title, p.rgba);
            if !p.detail.is_empty() {
                hud::text_centered(
                    &mut m,
                    SCREEN_W / 2.0,
                    120.0 + px * 7.0,
                    4.0,
                    &p.detail,
                    white,
                );
            }
        }
        if !self.connected {
            hud::text_centered(
                &mut m,
                SCREEN_W / 2.0,
                230.0,
                4.0,
                "CONNECT A GAMECUBE PAD TO PORT 1",
                rgb(255, 120, 100),
            );
        } else if self.clock < HINT_TIME {
            hud::text_centered(
                &mut m,
                SCREEN_W / 2.0,
                420.0,
                3.0,
                "A PUSH   B BRAKE   STICK STEER AND SPIN",
                white,
            );
            hud::text_centered(
                &mut m,
                SCREEN_W / 2.0,
                440.0,
                3.0,
                "C-STICK: PULL BACK THEN FLICK UP TO OLLIE",
                white,
            );
            hud::text_centered(
                &mut m,
                SCREEN_W / 2.0,
                456.0,
                3.0,
                "FLICK UP-LEFT KICKFLIP / UP-RIGHT HEELFLIP",
                white,
            );
        }
        gfx.draw_2d(&m.verts);
    }
}
