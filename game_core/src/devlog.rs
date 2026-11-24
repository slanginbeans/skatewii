//! Development log lines: what happened in the game, worded for reading back after a session. The game
//! collects them in [`crate::Game::log`]; the platform writes them out (the Wii build to the session log on the
//! SD card, the PC preview to the terminal) and adds its own timing and memory lines.
//!
//! One line per thing that happened (a flick and what it was read as, a pop, a landing and the numbers
//! it was judged on, a bail and why, stance changes, grinds, manuals, camera pull-ins), plus a status line
//! a second with the skater's state and the stick ranges seen (for tuning the C-stick).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::flick::FlickOut;
use crate::gesture;
use crate::input::PadRaw;
use crate::math::PI;
use crate::skater::{Event, Mode, Skater, Stance};

fn deg(rad: f32) -> f32 {
    rad * 180.0 / PI
}

/// Smallest and largest raw value seen on one stick axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Range {
    lo: i8,
    hi: i8,
}

impl Range {
    const EMPTY: Range = Range { lo: 0, hi: 0 };
    fn add(&mut self, v: i8) {
        self.lo = self.lo.min(v);
        self.hi = self.hi.max(v);
    }
}

#[derive(Clone, Debug)]
pub struct DevLog {
    mode: Mode,
    switch: bool,
    fakie: bool,
    manual: bool,
    grinding: bool,
    sliding: bool,
    /// Raw stick ranges since the last status line: main x, main y, C x, C y.
    ranges: [Range; 4],
    trig: [u8; 2],
    /// Frames since the last status line, and the camera's pull-ins (walls) in that time.
    frames: u32,
    cam_pulls: u32,
    last_cam: f32,
    /// Flicks recognised and their mean strength since the last status line.
    flicks: u32,
}

impl Default for DevLog {
    fn default() -> Self {
        Self::new()
    }
}

impl DevLog {
    pub fn new() -> DevLog {
        DevLog {
            mode: Mode::Rolling,
            switch: false,
            fakie: false,
            manual: false,
            grinding: false,
            sliding: false,
            ranges: [Range::EMPTY; 4],
            trig: [0; 2],
            frames: 0,
            cam_pulls: 0,
            last_cam: 0.0,
            flicks: 0,
        }
    }

    /// One frame: lines for what changed. `cam` is the camera's boom length now.
    pub fn observe(
        &mut self,
        s: &Skater,
        raw: &PadRaw,
        flick: &FlickOut,
        events: &[Event],
        cam: f32,
        out: &mut Vec<String>,
    ) {
        self.frames += 1;
        for (r, v) in
            self.ranges
                .iter_mut()
                .zip([raw.stick_x, raw.stick_y, raw.cstick_x, raw.cstick_y])
        {
            r.add(v);
        }
        self.trig = [self.trig[0].max(raw.trig_l), self.trig[1].max(raw.trig_r)];
        if cam + 0.3 < self.last_cam {
            self.cam_pulls += 1;
        }
        self.last_cam = cam;

        if let Some(f) = flick.flick {
            self.flicks += 1;
            let read = gesture::select(f, s.flick_group(), s.mirrored());
            out.push(format!(
                "flick: {:?}{} strength {:.2} -> {}{} ({:?} group{})",
                f.trick,
                if f.nollie { " (nollie)" } else { "" },
                flick.strength,
                if read.nollie { "nollie " } else { "" },
                read.trick.name(),
                s.flick_group(),
                if s.mirrored() { ", mirrored" } else { "" },
            ));
        }
        for e in events {
            match e {
                Event::SteppedOff => out.push(format!(
                    "off the board at {:.1} m/s, facing {:.0} deg",
                    s.biped.speed,
                    s.biped.heading.to_degrees()
                )),
                Event::SteppedOn => out.push(format!("on the board at {:.1} m/s", s.speed)),
                Event::Popped(f) => out.push(format!(
                    "pop: {}{} at {:.1} m/s, up {:.2} m/s",
                    if f.nollie { "nollie " } else { "" },
                    f.trick.name(),
                    s.speed.abs().max(s.vel.horizontal().length()),
                    s.vel.y
                )),
                Event::Landed { name, points } => {
                    let detail = match (&s.landing, s.mode) {
                        (Some(l), Mode::Rolling) if l.time == 0.0 => format!(
                            " ({:?}, {:.0} deg off, {:.1} m/s down)",
                            l.kind,
                            deg(l.crooked),
                            l.impact
                        ),
                        _ => String::new(),
                    };
                    out.push(format!("landed: {name} +{points}{detail}"));
                }
                Event::Bailed => {
                    let why = s.bail_reason.unwrap_or("?");
                    let numbers = s.last_landing.map_or(String::new(), |c| {
                        format!(
                            " ({:.0} deg off of {:.0} allowed, {:.1} m/s down, {:.1} m/s along)",
                            deg(c.crooked),
                            deg(c.limit),
                            c.impact,
                            c.speed
                        )
                    });
                    out.push(format!("BAIL: {why}{numbers}"));
                }
                Event::Respawned => out.push(format!(
                    "respawned at ({:.1}, {:.1}, {:.1}) heading {:.0} deg{} ({} checkpoints left)",
                    s.pos.x,
                    s.pos.y,
                    s.pos.z,
                    deg(s.heading),
                    if s.switch { ", switch" } else { "" },
                    s.checkpoints.len()
                )),
                Event::RanOut => out.push(format!(
                    "RUNOUT: {} (stepped off, back on where it stops)",
                    s.bail_reason.unwrap_or("?")
                )),
                Event::ManualLost => out.push(String::from("manual: tipped over")),
                Event::Launched if s.hippie_air() => out.push(format!(
                    "hippie jump at {:.1} m/s, up {:.2} m/s",
                    s.vel.horizontal().length(),
                    s.vel.y
                )),
                Event::Launched => out.push(format!(
                    "launched off a lip at {:.1} m/s",
                    s.vel.horizontal().length()
                )),
                Event::Switched => out.push(String::from(
                    "stance: pushed from fakie, turned round to switch",
                )),
                Event::FootJump(k) => out.push(format!(
                    "{}: jumped off the planted foot at {:.1} m/s, up {:.2} m/s",
                    k.trick(),
                    s.vel.horizontal().length(),
                    s.vel.y
                )),
                Event::FootPlanted => out.push(format!(
                    "foot plant: {} in {:.2} s on a surface facing ({:.2}, {:.2}, {:.2})",
                    s.footplanted.unwrap_or("?"),
                    s.air_plant.map_or(0.0, |p| p.length()),
                    s.footplant.normal.x,
                    s.footplant.normal.y,
                    s.footplant.normal.z
                )),
                Event::FootReleased => out.push(format!(
                    "foot plant: off the foot at {:.1} m/s, up {:.2} m/s",
                    s.vel.horizontal().length(),
                    s.vel.y
                )),
            }
        }
        if s.switch != self.switch {
            out.push(format!(
                "stance: {} {}",
                if s.switch {
                    "riding switch"
                } else {
                    "riding natural"
                },
                match s.stance {
                    Stance::Goofy => "(goofy rider)",
                    Stance::Regular => "(regular rider)",
                }
            ));
            self.switch = s.switch;
        }
        if s.fakie != self.fakie {
            out.push(format!(
                "stance: {} at {:.1} m/s",
                if s.fakie {
                    "fakie (rolling tail first)"
                } else {
                    "no longer fakie"
                },
                s.speed.abs()
            ));
            self.fakie = s.fakie;
        }
        // While in a manual, how it balances (a few times a second): why manuals tip over.
        if let Some(m) = &s.manual
            && self.frames.is_multiple_of(15)
        {
            out.push(format!(
                "manual: balance {:+.2}, C-stick y {:+.2} (hold {:.2}..{:.2}, balanced near {:.2}), brake {:.2}",
                m.balance, flick.stick[1], 0.1, 0.9, crate::skater::MANUAL_BALANCED_AT, m.brake
            ));
        }
        let manual = s.manual.is_some();
        if manual != self.manual {
            out.push(match &s.manual {
                Some(m) => format!(
                    "manual: {} at {:.1} m/s",
                    if m.nose { "nose" } else { "tail" },
                    s.speed
                ),
                None => String::from("manual: off"),
            });
            self.manual = manual;
        }
        let grinding = s.grind.is_some();
        if grinding != self.grinding {
            if let Some(g) = &s.grind {
                out.push(format!(
                    "grind: {} ({}, {:?}, {:?}) on rail {} ({}) at {:.1} m/s",
                    g.display(),
                    g.graph_name(),
                    g.family,
                    g.entry,
                    g.rail,
                    g.class.name(),
                    g.speed
                ));
            }
            if !grinding {
                out.push(format!(
                    "grind: off ({})",
                    s.grind_exit.unwrap_or(if s.is_airborne() {
                        "in the air"
                    } else {
                        "wiped out"
                    })
                ));
            }
            self.grinding = grinding;
        }
        let sliding = s.slide.is_some();
        if sliding != self.sliding {
            if sliding {
                out.push(format!("powerslide at {:.1} m/s", s.speed.abs()));
            }
            self.sliding = sliding;
        }
        if core::mem::discriminant(&s.mode) != core::mem::discriminant(&self.mode) {
            self.mode = s.mode;
        }
    }

    /// The status line (call about once a second): the skater's state and what the sticks did since the
    /// last one. Resets the ranges.
    pub fn status(&mut self, s: &Skater, cam: f32) -> String {
        let r = &self.ranges;
        let mode = match s.mode {
            Mode::Rolling => "rolling",
            Mode::Air { .. } => "air",
            Mode::Bailed { .. } => "bailed",
            Mode::Runout { .. } => "runout",
            Mode::Grind => "grind",
            Mode::Walk if s.biped.air.is_some() => "walk (air)",
            Mode::Walk => "walk",
        };
        let line = format!(
            "status: {mode} at ({:.1}, {:.1}, {:.1}) heading {:.0} deg, {:.1} m/s, {:?}{}{} | cam {:.1} m, {} pull-ins | \
             sticks seen: main x {}..{} y {}..{}, C x {}..{} y {}..{}, triggers {} {} | {} flicks",
            s.pos.x,
            s.pos.y,
            s.pos.z,
            deg(s.heading),
            s.speed,
            s.stance,
            if s.switch { " switch" } else { "" },
            if s.fakie { " fakie" } else { "" },
            cam,
            self.cam_pulls,
            r[0].lo,
            r[0].hi,
            r[1].lo,
            r[1].hi,
            r[2].lo,
            r[2].hi,
            r[3].lo,
            r[3].hi,
            self.trig[0],
            self.trig[1],
            self.flicks,
        );
        // A pad that reaches further than the game assumes (the outer part reads as fully over).
        let main = r[0].hi.max(-r[0].lo).max(r[1].hi).max(-r[1].lo) as f32;
        let c = r[2].hi.max(-r[2].lo).max(r[3].hi).max(-r[3].lo) as f32;
        let mut line = line;
        if main > crate::input::MAIN_RANGE * 1.08 || c > crate::input::C_RANGE * 1.08 {
            line.push_str(&format!(
                " | NOTE: this pad reaches main {main:.0} / C {c:.0}, beyond the {:.0} / {:.0} the game assumes",
                crate::input::MAIN_RANGE,
                crate::input::C_RANGE
            ));
        }
        self.ranges = [Range::EMPTY; 4];
        self.trig = [0; 2];
        self.frames = 0;
        self.cam_pulls = 0;
        self.flicks = 0;
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::{Flick, Trick};
    use crate::park::Park;

    #[test]
    fn lines_for_flicks_bails_and_stance() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        let mut log = DevLog::new();
        let mut out = Vec::new();
        let f = FlickOut {
            flick: Some(Flick::of(Trick::Kickflip)),
            strength: 0.8,
            ..FlickOut::default()
        };
        log.observe(
            &s,
            &PadRaw::default(),
            &f,
            &[Event::Popped(Flick::of(Trick::Kickflip))],
            2.0,
            &mut out,
        );
        assert!(
            out[0].starts_with("flick: Kickflip strength 0.80 -> KICKFLIP"),
            "{out:?}"
        );
        assert!(out[1].starts_with("pop: KICKFLIP"), "{out:?}");
        out.clear();
        s.bail_reason = Some("landed sideways");
        s.switch = true;
        log.observe(
            &s,
            &PadRaw::default(),
            &FlickOut::default(),
            &[Event::Bailed],
            0.8,
            &mut out,
        );
        assert!(out.iter().any(|l| l == "BAIL: landed sideways"), "{out:?}");
        assert!(
            out.iter().any(|l| l.starts_with("stance: riding switch")),
            "{out:?}"
        );
    }

    #[test]
    fn status_reports_stick_ranges_and_resets() {
        let park = Park::demo();
        let s = Skater::spawn(&park);
        let mut log = DevLog::new();
        let mut out = Vec::new();
        for (x, y) in [(-70i8, 3i8), (66, -59)] {
            let raw = PadRaw {
                cstick_x: x,
                cstick_y: y,
                ..PadRaw::default()
            };
            log.observe(&s, &raw, &FlickOut::default(), &[], 2.0, &mut out);
        }
        let line = log.status(&s, 2.0);
        assert!(line.contains("C x -70..66 y -59..3"), "{line}");
        assert!(log.status(&s, 2.0).contains("C x 0..0 y 0..0"));
    }
}
