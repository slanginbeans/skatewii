//! Locations and the menus: the start menu (choose where to ride), the pause menu (START while riding) and
//! the location index the importer writes (`data/parks.txt`, see docs/importing.md).
//!
//! The menus only decide; the platform does what needs files (reading a park pack, saving the last
//! location, quitting to the loader) when the game hands it a [`Request`].

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::gfx::{MeshBuilder, SCREEN_H, SCREEN_W, rgb};
use crate::hud;
use crate::input::{MAIN_RANGE, PadRaw, btn};
use crate::tuning::Difficulty;
use crate::tuning::mode::{DEFAULT_TRUCK_TIGHTNESS, TRUCK_TIGHTNESS_STEPS};

/// One place to ride: an imported park pack, a streamed district (its index pack), or the built-in demo
/// park (no file).
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    /// Short name without spaces (the disc's district name, or `demo`): what the settings file and the
    /// index's `default` line refer to.
    pub id: String,
    /// Shown in the menus.
    pub name: String,
    /// The pack, relative to the data folder (`parks/BlackBoxPark.fgpak`); empty for the demo park.
    pub file: String,
    /// The pack's size in bytes (for the loading message; 0: unknown). A district's: all its cells.
    pub bytes: u32,
    /// A district streamed in cells (`game_core::district`), started at this start of its index: `file`
    /// is its index pack (`city/DownTown/index.fgpak`), the cells next to it. A district has one location
    /// per start.
    pub district: Option<u8>,
}

impl Location {
    /// The built-in demo park, always the last location in the menus.
    pub fn demo() -> Location {
        Location {
            id: DEMO_ID.into(),
            name: "Demo park".into(),
            file: String::new(),
            bytes: 0,
            district: None,
        }
    }

    pub fn is_demo(&self) -> bool {
        self.file.is_empty()
    }
}

pub const DEMO_ID: &str = "demo";

/// The importer's location index (`parks.txt`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub locations: Vec<Location>,
    /// The `default` line's location (the one the build was asked for with `FG_PARK`), if listed.
    pub default: Option<usize>,
}

/// Reads an index. Lines (anything else, and `#` comments, are skipped):
///
/// ```text
/// default <id>
/// park <id> <file> <bytes> <display name ...>
/// district <id> <index file> <bytes> <start> <display name ...>
/// ```
///
/// Files must be plain relative paths inside the data folder (no `:`, no `..`, no leading `/`).
pub fn parse_index(text: &str) -> Index {
    let mut index = Index::default();
    let mut default = None;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("default") => default = words.next().map(|s| s.to_string()),
            Some(kind @ ("park" | "district")) => {
                let (Some(id), Some(file), Some(bytes)) =
                    (words.next(), words.next(), words.next())
                else {
                    continue;
                };
                let Ok(bytes) = bytes.parse::<u32>() else {
                    continue;
                };
                let district = if kind == "district" {
                    match words.next().and_then(|s| s.parse::<u8>().ok()) {
                        Some(s) => Some(s),
                        None => continue,
                    }
                } else {
                    None
                };
                let safe = !file.contains(':') && !file.contains("..") && !file.starts_with('/');
                if !safe || id == DEMO_ID || index.locations.iter().any(|l| l.id == id) {
                    continue;
                }
                let name = words.collect::<Vec<_>>().join(" ");
                index.locations.push(Location {
                    id: id.into(),
                    name: if name.is_empty() { id.into() } else { name },
                    file: file.into(),
                    bytes,
                    district,
                });
            }
            _ => {}
        }
    }
    index.default = default.and_then(|d| index.locations.iter().position(|l| l.id == d));
    index
}

/// Writes an index (the importer's side of [`parse_index`]).
pub fn write_index(locations: &[Location], default: Option<&str>) -> String {
    let mut out = String::from("# flatground locations, written by fg_import build\n");
    if let Some(d) = default {
        out += &format!("default {d}\n");
    }
    for l in locations.iter().filter(|l| !l.is_demo()) {
        out += &match l.district {
            Some(s) => format!("district {} {} {} {s} {}\n", l.id, l.file, l.bytes, l.name),
            None => format!("park {} {} {} {}\n", l.id, l.file, l.bytes, l.name),
        };
    }
    out
}

/// Skate 3's Camera Angle setting (Game Settings, "High" / "Low"; SK8 docs/hails-additions/
/// 21-low-camera.md): which half of the camera graph runs. Skate 3's camera preferences start at 0, the
/// low camera (its first boot asks the player), and flatground's camera has always been the low one:
/// Low is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraAngle {
    #[default]
    Low,
    High,
}

impl CameraAngle {
    /// The camera graph's `IsCameraTypeActive` type: 0 Low, 1 High.
    pub fn graph_type(self) -> u32 {
        match self {
            CameraAngle::Low => crate::camera::select::LOW,
            CameraAngle::High => crate::camera::select::HIGH,
        }
    }

    pub fn of_graph_type(t: u32) -> CameraAngle {
        if t == crate::camera::select::HIGH {
            CameraAngle::High
        } else {
            CameraAngle::Low
        }
    }

    /// As the settings file and the menu write it.
    pub fn name(self) -> &'static str {
        match self {
            CameraAngle::Low => "low",
            CameraAngle::High => "high",
        }
    }

    pub fn other(self) -> CameraAngle {
        match self {
            CameraAngle::Low => CameraAngle::High,
            CameraAngle::High => CameraAngle::Low,
        }
    }
}

/// What the settings file in the app folder (`settings.txt`) remembers: `location <id>`, the last location
/// chosen, `draw_distance <m>`, a district's draw distance, `camera low|high`, the camera angle, and
/// `volume <0..10>`, the master volume, `difficulty easy|normal|hardcore`, Skate 3's Play Mode, and
/// `trucks <0..10>`, the truck tightness in tenths. A line it doesn't know is skipped and one it lacks
/// keeps the default (Skate 3's Normal, trucks 0.7), so older files still load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub location: Option<String>,
    pub draw_distance: Option<u16>,
    pub camera: Option<CameraAngle>,
    pub volume: Option<u8>,
    pub difficulty: Option<Difficulty>,
    pub trucks: Option<u8>,
}

pub fn parse_settings(text: &str) -> Settings {
    let mut s = Settings::default();
    for l in text.lines() {
        let mut w = l.split_whitespace();
        match (w.next(), w.next()) {
            (Some("location"), Some(id)) => s.location = Some(id.to_string()),
            (Some("draw_distance"), Some(d)) => s.draw_distance = d.parse().ok(),
            (Some("camera"), Some("low")) => s.camera = Some(CameraAngle::Low),
            (Some("camera"), Some("high")) => s.camera = Some(CameraAngle::High),
            (Some("volume"), Some(v)) => {
                s.volume = v
                    .parse::<u8>()
                    .ok()
                    .filter(|v| *v <= crate::sound::VOLUME_STEPS)
            }
            (Some("difficulty"), Some(d)) => s.difficulty = Difficulty::parse(d),
            (Some("trucks"), Some(t)) => {
                s.trucks = t.parse::<u8>().ok().filter(|t| *t <= TRUCK_TIGHTNESS_STEPS)
            }
            _ => {}
        }
    }
    s
}

pub fn write_settings(
    location: &str,
    draw_distance: u16,
    camera: CameraAngle,
    volume: u8,
    difficulty: Difficulty,
    trucks: u8,
) -> String {
    format!(
        "# flatground settings\nlocation {location}\ndraw_distance {draw_distance}\ncamera {}\nvolume {volume}\ndifficulty {}\ntrucks {trucks}\n",
        camera.name(),
        difficulty.key()
    )
}

/// The location to start in: the last one chosen (settings), else the index's default, else the first.
pub fn start_location(locations: &[Location], last: Option<&str>, default: Option<usize>) -> usize {
    last.and_then(|id| locations.iter().position(|l| l.id == id))
        .or(default)
        .unwrap_or(0)
        .min(locations.len().saturating_sub(1))
}

/// What the game asks the platform to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Ride location `n` of `Game::locations` (read its pack, or switch to the demo park).
    Load(usize),
    /// Back to the Homebrew Channel / loader.
    Quit,
}

/// What a menu decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Ride location `n` (the start menu, or Change location).
    Ride(usize),
    Resume,
    Respawn,
    Quit,
    /// The draw distance was changed to this (m); the menu stays open.
    DrawDistance(u16),
    /// The camera angle was changed to this; the menu stays open.
    Camera(CameraAngle),
    /// The master volume was changed to this (0..=10); the menu stays open.
    Volume(u8),
    /// The difficulty was changed to this; the menu stays open.
    Difficulty(Difficulty),
    /// The truck tightness was changed to this (tenths, 0..=10); the menu stays open.
    Trucks(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// At boot: the locations, nothing to go back to.
    Start,
    /// START while riding.
    Pause,
    /// The locations, from the pause menu (B goes back to it).
    Locations,
}

pub const PAUSE_ITEMS: [&str; 9] = [
    "RESUME",
    "CHANGE LOCATION",
    "DRAW DISTANCE",
    "CAMERA",
    "VOLUME",
    "DIFFICULTY",
    "TRUCKS",
    "RESPAWN",
    "QUIT TO LOADER",
];
/// The pause menu's draw distance row: left and right change it by [`DISTANCE_STEP`].
const DISTANCE_ROW: usize = 2;
/// The camera angle row: left, right or A switch between Low and High.
const CAMERA_ROW: usize = 3;
/// The master volume row: left and right step it by one (0..=10).
const VOLUME_ROW: usize = 4;
/// Skate 3's Play Mode row: left and right step Easy, Normal, Hardcore, wrapping round as Skate 3's
/// does.
const DIFFICULTY_ROW: usize = 5;
/// The truck tightness (Skate 3's board setup): left and right step it by 0.1 (0..=1).
const TRUCKS_ROW: usize = 6;
const RESPAWN_ROW: usize = 7;
pub const DISTANCE_STEP: u16 = 10;

/// Holding a direction moves again after this long (s), then every [`REPEAT`].
const REPEAT_DELAY: f32 = 0.4;
const REPEAT: f32 = 0.12;
/// The main stick counts as up or down past this share of its range.
const STICK: f32 = 0.6;

/// A menu on screen. It reads the raw pad itself (edge-triggered against what was held when it opened, so
/// the button that opened it does nothing).
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub screen: Screen,
    pub cursor: usize,
    /// The pause menu's cursor, kept while the location list is open.
    pause_cursor: usize,
    prev_held: u32,
    /// The direction held last frame (-1 up, 1 down, 0 none) and how long until it repeats.
    dir: i8,
    repeat_in: f32,
    /// The draw distance shown (m), and whether left or right was held last frame.
    pub draw_distance: u16,
    side: i8,
    /// The camera angle shown.
    pub camera: CameraAngle,
    /// The master volume shown (0..=10).
    pub volume: u8,
    /// The difficulty shown.
    pub difficulty: Difficulty,
    /// The truck tightness shown (tenths).
    pub trucks: u8,
}

impl Menu {
    /// The start menu with `current` highlighted; `held`: the buttons down now.
    pub fn start(current: usize, held: u32) -> Menu {
        Menu {
            screen: Screen::Start,
            cursor: current,
            pause_cursor: 0,
            prev_held: held,
            dir: 0,
            repeat_in: 0.0,
            draw_distance: crate::world::DRAW_DISTANCE as u16,
            side: 0,
            camera: CameraAngle::default(),
            volume: crate::sound::DEFAULT_VOLUME,
            difficulty: Difficulty::default(),
            trucks: DEFAULT_TRUCK_TIGHTNESS,
        }
    }

    /// The pause menu, showing `draw_distance` (m).
    pub fn pause(held: u32, draw_distance: u16) -> Menu {
        Menu {
            screen: Screen::Pause,
            cursor: 0,
            draw_distance,
            ..Menu::start(0, held)
        }
    }

    fn items(&self, locations: usize) -> usize {
        match self.screen {
            Screen::Pause => PAUSE_ITEMS.len(),
            _ => locations,
        }
    }

    /// One displayed frame of `dt` seconds with the pad as read; `locations`: how many there are;
    /// `current`: the one being ridden (highlighted when the location list opens).
    pub fn update(
        &mut self,
        raw: &PadRaw,
        dt: f32,
        locations: usize,
        current: usize,
    ) -> Option<Choice> {
        let held = raw.buttons;
        let pressed = held & !self.prev_held;
        self.prev_held = held;
        // Up or down: the d-pad, else the main stick.
        let stick = raw.stick_y as f32 / MAIN_RANGE;
        let dir: i8 = if held & btn::DPAD_UP != 0 || stick > STICK {
            -1
        } else if held & btn::DPAD_DOWN != 0 || stick < -STICK {
            1
        } else {
            0
        };
        let n = self.items(locations).max(1);
        let mut step = 0;
        if dir != 0 && dir != self.dir {
            step = dir;
            self.repeat_in = REPEAT_DELAY;
        } else if dir != 0 {
            self.repeat_in -= dt;
            if self.repeat_in <= 0.0 {
                step = dir;
                self.repeat_in += REPEAT;
            }
        }
        self.dir = dir;
        if step != 0 {
            self.cursor = if step < 0 {
                (self.cursor + n - 1) % n
            } else {
                (self.cursor + 1) % n
            };
        }
        let accept = pressed & btn::A != 0;
        let back = pressed & btn::B != 0;
        let start = pressed & btn::START != 0;
        // Left or right on the draw distance row: one step a press (d-pad or stick).
        let sx = raw.stick_x as f32 / MAIN_RANGE;
        let side: i8 = if held & btn::DPAD_LEFT != 0 || sx < -STICK {
            -1
        } else if held & btn::DPAD_RIGHT != 0 || sx > STICK {
            1
        } else {
            0
        };
        let turned = side != 0 && side != self.side;
        self.side = side;
        if self.screen == Screen::Pause && self.cursor == DISTANCE_ROW && turned {
            let lo = crate::world::MIN_DRAW_DISTANCE as i32;
            let hi = crate::world::MAX_DRAW_DISTANCE as i32;
            let d = self.draw_distance as i32 + side as i32 * DISTANCE_STEP as i32;
            let d = d.clamp(lo, hi) as u16;
            if d != self.draw_distance {
                self.draw_distance = d;
                return Some(Choice::DrawDistance(d));
            }
        }
        if self.screen == Screen::Pause && self.cursor == VOLUME_ROW && turned {
            let v = (self.volume as i32 + side as i32).clamp(0, crate::sound::VOLUME_STEPS as i32);
            if v as u8 != self.volume {
                self.volume = v as u8;
                return Some(Choice::Volume(self.volume));
            }
        }
        if self.screen == Screen::Pause && self.cursor == DIFFICULTY_ROW && turned {
            self.difficulty = self.difficulty.step(side);
            return Some(Choice::Difficulty(self.difficulty));
        }
        if self.screen == Screen::Pause && self.cursor == TRUCKS_ROW && turned {
            let t = (self.trucks as i32 + side as i32).clamp(0, TRUCK_TIGHTNESS_STEPS as i32);
            if t as u8 != self.trucks {
                self.trucks = t as u8;
                return Some(Choice::Trucks(self.trucks));
            }
        }
        if self.screen == Screen::Pause && self.cursor == CAMERA_ROW && (turned || accept) {
            self.camera = self.camera.other();
            return Some(Choice::Camera(self.camera));
        }
        match self.screen {
            Screen::Start => (accept || start).then_some(Choice::Ride(self.cursor)),
            Screen::Locations if accept => Some(Choice::Ride(self.cursor)),
            Screen::Locations if back => {
                self.screen = Screen::Pause;
                self.cursor = self.pause_cursor;
                None
            }
            Screen::Locations => start.then_some(Choice::Resume),
            Screen::Pause if back || start => Some(Choice::Resume),
            Screen::Pause if accept => match self.cursor {
                0 => Some(Choice::Resume),
                1 => {
                    self.pause_cursor = 1;
                    self.screen = Screen::Locations;
                    self.cursor = current.min(locations.saturating_sub(1));
                    None
                }
                DISTANCE_ROW | CAMERA_ROW | VOLUME_ROW | DIFFICULTY_ROW | TRUCKS_ROW => None,
                RESPAWN_ROW => Some(Choice::Respawn),
                _ => Some(Choice::Quit),
            },
            Screen::Pause => None,
        }
    }

    /// The menu over the frozen scene. `names`: the locations' names; `current`: the one being ridden.
    pub fn draw(&self, m: &mut MeshBuilder, names: &[&str], current: usize) {
        let (title, items): (&str, Vec<String>) = match self.screen {
            Screen::Pause => (
                "PAUSED",
                PAUSE_ITEMS
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        if i == DISTANCE_ROW {
                            format!("< {s} {} M >", self.draw_distance)
                        } else if i == CAMERA_ROW {
                            format!("< {s} {} >", self.camera.name().to_ascii_uppercase())
                        } else if i == VOLUME_ROW {
                            format!("< {s} {} >", self.volume)
                        } else if i == DIFFICULTY_ROW {
                            format!("< {s} {} >", self.difficulty.key().to_ascii_uppercase())
                        } else if i == TRUCKS_ROW {
                            format!(
                                "< {s} {}.{} >",
                                self.trucks / TRUCK_TIGHTNESS_STEPS,
                                self.trucks % TRUCK_TIGHTNESS_STEPS
                            )
                        } else {
                            s.to_string()
                        }
                    })
                    .collect(),
            ),
            Screen::Start | Screen::Locations => (
                "CHOOSE A LOCATION",
                names.iter().map(|n| n.to_ascii_uppercase()).collect(),
            ),
        };
        let row = 26.0;
        // A long list scrolls: the rows around the cursor, and a mark where more are above or below.
        let (from, to) = shown(items.len(), self.cursor);
        let h = 110.0 + row * (to - from) as f32;
        let top = ((SCREEN_H - h) / 2.0).max(16.0);
        m.rect_2d(0.0, 0.0, SCREEN_W, SCREEN_H, 0x0000_0060);
        m.rect_2d(80.0, top, SCREEN_W - 160.0, h, 0x1a1e_24e0);
        if self.screen == Screen::Start {
            hud::text_centered(
                m,
                SCREEN_W / 2.0,
                top + 14.0,
                5.0,
                "FLATGROUND",
                rgb(255, 230, 90),
            );
        }
        let title_y = top
            + if self.screen == Screen::Start {
                46.0
            } else {
                20.0
            };
        hud::text_centered(m, SCREEN_W / 2.0, title_y, 3.0, title, rgb(200, 220, 255));
        let first = title_y + 30.0;
        let more = rgb(120, 140, 170);
        if from > 0 {
            hud::text_centered(m, SCREEN_W / 2.0, first - 14.0, 2.0, "MORE", more);
        }
        if to < items.len() {
            let y = first + row * (to - from) as f32 - 4.0;
            hud::text_centered(m, SCREEN_W / 2.0, y, 2.0, "MORE", more);
        }
        for (i, item) in items.iter().enumerate().take(to).skip(from) {
            let y = first + row * (i - from) as f32;
            let on = i == self.cursor;
            if on {
                m.rect_2d(96.0, y - 6.0, SCREEN_W - 192.0, row - 2.0, 0x3c64_a0ff);
            }
            // A marker by the location being ridden.
            if self.screen != Screen::Pause && i == current {
                m.rect_2d(108.0, y + 2.0, 8.0, 8.0, rgb(255, 230, 90));
            }
            let colour = if on {
                rgb(255, 255, 255)
            } else {
                rgb(190, 190, 190)
            };
            hud::text_centered(m, SCREEN_W / 2.0, y, 3.0, item, colour);
        }
        let help = match self.screen {
            Screen::Start => "UP/DOWN CHOOSE  A RIDE",
            Screen::Pause
                if matches!(
                    self.cursor,
                    DISTANCE_ROW | CAMERA_ROW | VOLUME_ROW | DIFFICULTY_ROW | TRUCKS_ROW
                ) =>
            {
                "LEFT/RIGHT CHANGE  B/START RESUME"
            }
            Screen::Pause => "A SELECT  B/START RESUME",
            Screen::Locations => "A RIDE  B BACK",
        };
        hud::text_centered(
            m,
            SCREEN_W / 2.0,
            top + h - 24.0,
            2.5,
            help,
            rgb(170, 170, 170),
        );
    }
}

/// Most rows a menu shows at once.
pub const ROWS: usize = 12;

/// The rows a menu of `n` shows with the cursor on `cursor` (`from..to`): all of them when they fit, else
/// [`ROWS`] with the cursor in the middle where it can be.
pub fn shown(n: usize, cursor: usize) -> (usize, usize) {
    if n <= ROWS {
        return (0, n);
    }
    let from = cursor.saturating_sub(ROWS / 2).min(n - ROWS);
    (from, from + ROWS)
}

/// Hands-off play through every location (`switch <seconds>` in `autoplay.txt`): which location comes
/// next, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct LocationTour {
    every: u32,
    frames: u32,
    /// The location last switched to (the game may be on the demo park instead, when it failed to load).
    at: Option<usize>,
}

impl LocationTour {
    /// From `autoplay.txt`'s text: `switch <seconds>` turns it on.
    pub fn from_autoplay(text: &str) -> Option<LocationTour> {
        text.lines().find_map(|l| {
            let mut w = l.split_whitespace();
            if w.next() != Some("switch") {
                return None;
            }
            let s: f32 = w.next()?.parse().ok()?;
            (s >= 1.0).then(|| LocationTour::every((s * 60.0) as u32))
        })
    }

    /// Every `frames` displayed frames.
    pub fn every(frames: u32) -> LocationTour {
        LocationTour {
            every: frames.max(1),
            frames: 0,
            at: None,
        }
    }

    /// One displayed frame: the location to switch to when it is time (the one after `current`, round
    /// all `count`).
    pub fn next(&mut self, current: usize, count: usize) -> Option<usize> {
        self.frames += 1;
        if self.frames < self.every || count < 2 {
            return None;
        }
        self.frames = 0;
        // On from the location last tried, so one that fails to load (the game falls back to the demo park,
        // the last) doesn't send the tour back to the first every time.
        let next = (self.at.unwrap_or(current) + 1) % count;
        self.at = Some(next);
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn pad(buttons: u32) -> PadRaw {
        PadRaw {
            buttons,
            ..Default::default()
        }
    }

    fn stick(y: i8) -> PadRaw {
        PadRaw {
            stick_y: y,
            ..Default::default()
        }
    }

    const DT: f32 = 1.0 / 60.0;

    /// Presses and releases `b`, returning what the menu decided.
    fn tap(m: &mut Menu, b: u32, n: usize, cur: usize) -> Option<Choice> {
        let c = m.update(&pad(b), DT, n, cur);
        c.or(m.update(&pad(0), DT, n, cur))
    }

    #[test]
    fn start_menu_moves_wraps_and_rides() {
        // Opened with A still held from somewhere: no ride until it is pressed again.
        let mut m = Menu::start(1, btn::A);
        assert_eq!(m.update(&pad(btn::A), DT, 3, 1), None);
        assert_eq!(tap(&mut m, btn::DPAD_DOWN, 3, 1), None);
        assert_eq!(m.cursor, 2);
        assert_eq!(tap(&mut m, btn::DPAD_DOWN, 3, 1), None);
        assert_eq!(m.cursor, 0, "wraps to the top");
        assert_eq!(tap(&mut m, btn::DPAD_UP, 3, 1), None);
        assert_eq!(m.cursor, 2, "wraps to the bottom");
        // B has nowhere to go back to.
        assert_eq!(tap(&mut m, btn::B, 3, 1), None);
        assert_eq!(tap(&mut m, btn::A, 3, 1), Some(Choice::Ride(2)));
        let mut m = Menu::start(0, 0);
        assert_eq!(tap(&mut m, btn::START, 3, 0), Some(Choice::Ride(0)));
    }

    #[test]
    fn the_stick_moves_once_then_repeats_while_held() {
        let mut m = Menu::start(0, 0);
        m.update(&stick(-90), DT, 9, 0);
        assert_eq!(m.cursor, 1);
        // A little over the repeat delay: still one step.
        for _ in 0..20 {
            m.update(&stick(-90), DT, 9, 0);
        }
        assert_eq!(m.cursor, 1);
        for _ in 0..10 {
            m.update(&stick(-90), DT, 9, 0);
        }
        assert!(m.cursor >= 2, "repeats after the delay: {}", m.cursor);
        // A light touch does nothing; up goes back.
        let c = m.cursor;
        m.update(&stick(30), DT, 9, 0);
        assert_eq!(m.cursor, c);
        m.update(&stick(90), DT, 9, 0);
        assert_eq!(m.cursor, c - 1);
    }

    #[test]
    fn pause_menu_items_and_the_location_list() {
        let mut m = Menu::pause(btn::START, 90);
        // The START that opened it is still held: nothing.
        assert_eq!(m.update(&pad(btn::START), DT, 4, 2), None);
        assert_eq!(tap(&mut m, btn::A, 4, 2), Some(Choice::Resume));
        let mut m = Menu::pause(0, 90);
        assert_eq!(tap(&mut m, btn::START, 4, 2), Some(Choice::Resume));
        let mut m = Menu::pause(0, 90);
        assert_eq!(tap(&mut m, btn::B, 4, 2), Some(Choice::Resume));
        // Change location: the list opens on the location being ridden; B comes back.
        let mut m = Menu::pause(0, 90);
        tap(&mut m, btn::DPAD_DOWN, 4, 2);
        assert_eq!(tap(&mut m, btn::A, 4, 2), None);
        assert_eq!((m.screen, m.cursor), (Screen::Locations, 2));
        assert_eq!(tap(&mut m, btn::B, 4, 2), None);
        assert_eq!((m.screen, m.cursor), (Screen::Pause, 1));
        tap(&mut m, btn::A, 4, 2);
        tap(&mut m, btn::DPAD_UP, 4, 2);
        assert_eq!(tap(&mut m, btn::A, 4, 2), Some(Choice::Ride(1)));
        // Respawn and quit.
        let mut m = Menu::pause(0, 90);
        for _ in 0..RESPAWN_ROW {
            tap(&mut m, btn::DPAD_DOWN, 4, 0);
        }
        assert_eq!(tap(&mut m, btn::A, 4, 0), Some(Choice::Respawn));
        let mut m = Menu::pause(0, 90);
        tap(&mut m, btn::DPAD_UP, 4, 0);
        assert_eq!(tap(&mut m, btn::A, 4, 0), Some(Choice::Quit));
    }

    #[test]
    fn the_draw_distance_row_steps_with_left_and_right() {
        let mut m = Menu::pause(0, 90);
        tap(&mut m, btn::DPAD_DOWN, 4, 0);
        tap(&mut m, btn::DPAD_DOWN, 4, 0);
        assert_eq!(m.cursor, DISTANCE_ROW);
        // A does nothing there; right and left step it, once a press, and the menu stays open.
        assert_eq!(tap(&mut m, btn::A, 4, 0), None);
        assert_eq!(
            tap(&mut m, btn::DPAD_RIGHT, 4, 0),
            Some(Choice::DrawDistance(100))
        );
        assert_eq!(
            m.update(&pad(btn::DPAD_LEFT), DT, 4, 0),
            Some(Choice::DrawDistance(90))
        );
        assert_eq!(
            m.update(&pad(btn::DPAD_LEFT), DT, 4, 0),
            None,
            "held: one step"
        );
        let left = PadRaw {
            stick_x: -90,
            ..Default::default()
        };
        m.update(&pad(0), DT, 4, 0);
        assert_eq!(m.update(&left, DT, 4, 0), Some(Choice::DrawDistance(80)));
        // It stops at the ends of the range.
        let mut m = Menu::pause(0, crate::world::MAX_DRAW_DISTANCE as u16);
        m.cursor = DISTANCE_ROW;
        assert_eq!(tap(&mut m, btn::DPAD_RIGHT, 4, 0), None);
        // Left and right elsewhere do nothing.
        let mut m = Menu::pause(0, 90);
        assert_eq!(tap(&mut m, btn::DPAD_RIGHT, 4, 0), None);
        assert_eq!(m.draw_distance, 90);
        // The row shows the distance.
        let mut mesh = MeshBuilder::new();
        Menu::pause(0, 70).draw(&mut mesh, &[], 0);
        let mut other = MeshBuilder::new();
        Menu::pause(0, 120).draw(&mut other, &[], 0);
        assert_ne!(mesh.verts, other.verts);
    }

    #[test]
    fn the_volume_row_steps_from_0_to_10() {
        let mut m = Menu::pause(0, 90);
        assert_eq!(m.volume, crate::sound::DEFAULT_VOLUME);
        for _ in 0..VOLUME_ROW {
            tap(&mut m, btn::DPAD_DOWN, 4, 0);
        }
        assert_eq!(m.cursor, VOLUME_ROW);
        assert_eq!(tap(&mut m, btn::DPAD_RIGHT, 4, 0), Some(Choice::Volume(9)));
        assert_eq!(tap(&mut m, btn::DPAD_RIGHT, 4, 0), Some(Choice::Volume(10)));
        assert_eq!(tap(&mut m, btn::DPAD_RIGHT, 4, 0), None, "10 is the most");
        m.volume = 0;
        assert_eq!(tap(&mut m, btn::DPAD_LEFT, 4, 0), None, "0 is the least");
        // A does nothing on it; the row shows the level.
        assert_eq!(tap(&mut m, btn::A, 4, 0), None);
        let mut a = MeshBuilder::new();
        m.draw(&mut a, &[], 0);
        m.volume = 7;
        let mut b = MeshBuilder::new();
        m.draw(&mut b, &[], 0);
        assert_ne!(a.verts, b.verts);
    }

    #[test]
    fn the_camera_row_switches_between_low_and_high() {
        let mut m = Menu::pause(0, 90);
        assert_eq!(m.camera, CameraAngle::Low, "Skate 3's default");
        for _ in 0..3 {
            tap(&mut m, btn::DPAD_DOWN, 4, 0);
        }
        assert_eq!(m.cursor, CAMERA_ROW);
        assert_eq!(
            tap(&mut m, btn::DPAD_RIGHT, 4, 0),
            Some(Choice::Camera(CameraAngle::High))
        );
        assert_eq!(
            tap(&mut m, btn::DPAD_LEFT, 4, 0),
            Some(Choice::Camera(CameraAngle::Low))
        );
        assert_eq!(
            tap(&mut m, btn::A, 4, 0),
            Some(Choice::Camera(CameraAngle::High))
        );
        // The row shows the angle.
        let mut high = MeshBuilder::new();
        m.draw(&mut high, &[], 0);
        m.camera = CameraAngle::Low;
        let mut low = MeshBuilder::new();
        m.draw(&mut low, &[], 0);
        assert_ne!(high.verts, low.verts);
        assert_eq!(
            CameraAngle::of_graph_type(CameraAngle::High.graph_type()),
            CameraAngle::High
        );
        assert_eq!(
            CameraAngle::of_graph_type(CameraAngle::Low.graph_type()),
            CameraAngle::Low
        );
    }

    #[test]
    fn the_difficulty_row_cycles_like_skate_3s_play_mode() {
        let mut m = Menu::pause(0, 90);
        assert_eq!(m.difficulty, Difficulty::Normal, "Skate 3's default");
        for _ in 0..DIFFICULTY_ROW {
            tap(&mut m, btn::DPAD_DOWN, 4, 0);
        }
        assert_eq!(m.cursor, DIFFICULTY_ROW);
        let right = |m: &mut Menu| tap(m, btn::DPAD_RIGHT, 4, 0);
        assert_eq!(
            right(&mut m),
            Some(Choice::Difficulty(Difficulty::Hardcore))
        );
        assert_eq!(
            right(&mut m),
            Some(Choice::Difficulty(Difficulty::Easy)),
            "wraps"
        );
        assert_eq!(
            tap(&mut m, btn::DPAD_LEFT, 4, 0),
            Some(Choice::Difficulty(Difficulty::Hardcore))
        );
        // A does nothing there (Skate 3's row only takes left and right); the row shows the mode.
        assert_eq!(tap(&mut m, btn::A, 4, 0), None);
        let mut a = MeshBuilder::new();
        m.draw(&mut a, &[], 0);
        m.difficulty = Difficulty::Easy;
        let mut b = MeshBuilder::new();
        m.draw(&mut b, &[], 0);
        assert_ne!(a.verts, b.verts);
    }

    #[test]
    fn the_trucks_row_steps_by_a_tenth_from_loose_to_tight() {
        let mut m = Menu::pause(0, 90);
        assert_eq!(m.trucks, 7, "Skate 3's 0.7");
        for _ in 0..TRUCKS_ROW {
            tap(&mut m, btn::DPAD_DOWN, 4, 0);
        }
        assert_eq!(m.cursor, TRUCKS_ROW);
        for t in 8..=10 {
            assert_eq!(tap(&mut m, btn::DPAD_RIGHT, 4, 0), Some(Choice::Trucks(t)));
        }
        assert_eq!(
            tap(&mut m, btn::DPAD_RIGHT, 4, 0),
            None,
            "1.0 is the tightest"
        );
        m.trucks = 0;
        assert_eq!(tap(&mut m, btn::DPAD_LEFT, 4, 0), None, "0 is the loosest");
        assert_eq!(tap(&mut m, btn::A, 4, 0), None);
        let mut a = MeshBuilder::new();
        m.draw(&mut a, &[], 0);
        m.trucks = 7;
        let mut b = MeshBuilder::new();
        m.draw(&mut b, &[], 0);
        assert_ne!(a.verts, b.verts);
    }

    #[test]
    fn menus_draw_their_items() {
        let mut m = MeshBuilder::new();
        Menu::start(0, 0).draw(&mut m, &["Black Box Park", "Demo park"], 0);
        let start = m.verts.len();
        assert!(start > 0);
        let mut p = MeshBuilder::new();
        Menu::pause(0, 90).draw(&mut p, &[], 0);
        assert!(!p.verts.is_empty());
    }

    #[test]
    fn index_round_trip_and_bad_lines() {
        let list = vec![
            Location {
                id: "BlackBoxPark".into(),
                name: "Black Box Park".into(),
                file: "parks/BlackBoxPark.fgpak".into(),
                bytes: 1089632,
                district: None,
            },
            Location {
                id: "MegaPark".into(),
                name: "Mega Park".into(),
                file: "parks/MegaPark.fgpak".into(),
                bytes: 12,
                district: None,
            },
            Location {
                id: "DownTown".into(),
                name: "Downtown".into(),
                file: "city/DownTown/index.fgpak".into(),
                bytes: 170_000_000,
                district: Some(1),
            },
            Location::demo(),
        ];
        let text = write_index(&list, Some("MegaPark"));
        let index = parse_index(&text);
        assert_eq!(index.locations, list[..3]);
        assert_eq!(index.default, Some(1));
        let messy = "# comment\n\
                     park A a.fgpak 10 Name A\n\
                     park B sd:/x.fgpak 10 Escapes\n\
                     park C ../x.fgpak 10 Escapes\n\
                     park D /x.fgpak 10 Absolute\n\
                     park E e.fgpak notanumber Bad\n\
                     park A a2.fgpak 10 Duplicate\n\
                     park demo d.fgpak 10 Reserved\n\
                     park F\n\
                     park G g.fgpak 7\n\
                     district H h/index.fgpak 7 Name without a start\n\
                     default Nowhere\n\
                     something else\n";
        let index = parse_index(messy);
        let ids: Vec<_> = index.locations.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["A", "G"]);
        assert_eq!(index.locations[0].name, "Name A");
        assert_eq!(index.locations[1].name, "G", "no name: the id");
        assert_eq!(index.default, None);
        assert_eq!(parse_index(""), Index::default());
    }

    #[test]
    fn settings_and_the_start_location() {
        let text = write_settings(
            "MegaPark",
            70,
            CameraAngle::High,
            3,
            Difficulty::Hardcore,
            4,
        );
        let s = parse_settings(&text);
        assert_eq!(s.location.as_deref(), Some("MegaPark"));
        assert_eq!(s.draw_distance, Some(70));
        assert_eq!(s.camera, Some(CameraAngle::High));
        assert_eq!(s.volume, Some(3));
        assert_eq!(s.difficulty, Some(Difficulty::Hardcore));
        assert_eq!(s.trucks, Some(4));
        assert_eq!(parse_settings("volume 11\n").volume, None, "out of range");
        assert_eq!(parse_settings("volume 0\n").volume, Some(0));
        assert_eq!(parse_settings("trucks 11\n").trucks, None, "out of range");
        assert_eq!(parse_settings("trucks 0\n").trucks, Some(0));
        assert_eq!(parse_settings("difficulty motorized\n").difficulty, None);
        for d in Difficulty::ALL {
            let text = write_settings("A", 90, CameraAngle::Low, 8, d, 7);
            assert_eq!(parse_settings(&text).difficulty, Some(d));
        }
        assert_eq!(
            parse_settings(&write_settings(
                "A",
                90,
                CameraAngle::Low,
                8,
                Difficulty::Normal,
                7
            ))
            .camera,
            Some(CameraAngle::Low)
        );
        // A file from before the difficulty and trucks settings: Skate 3's Normal and 0.7.
        let old = parse_settings(
            "# flatground settings\nlocation DownTown\ndraw_distance 70\ncamera high\nvolume 6\n",
        );
        assert_eq!(
            (old.volume, old.difficulty, old.trucks),
            (Some(6), None, None)
        );
        assert_eq!(
            parse_settings("garbage\nlocation\ncamera sideways\n"),
            Settings::default()
        );
        // A file from before the draw distance setting.
        let old = parse_settings("# flatground settings\nlocation DownTown\n");
        assert_eq!(
            (old.location.as_deref(), old.draw_distance, old.camera),
            (Some("DownTown"), None, None)
        );
        // And one from before the camera setting: the default camera.
        let old = parse_settings("# flatground settings\nlocation DownTown\ndraw_distance 70\n");
        assert_eq!((old.draw_distance, old.camera), (Some(70), None));
        // And one from before the volume setting: the default volume.
        let old = parse_settings(
            "# flatground settings\nlocation DownTown\ndraw_distance 70\ncamera high\n",
        );
        assert_eq!((old.camera, old.volume), (Some(CameraAngle::High), None));
        let list = vec![
            Location {
                id: "A".into(),
                name: "A".into(),
                file: "a".into(),
                bytes: 0,
                district: None,
            },
            Location {
                id: "B".into(),
                name: "B".into(),
                file: "b".into(),
                bytes: 0,
                district: None,
            },
            Location::demo(),
        ];
        assert_eq!(start_location(&list, Some("B"), Some(0)), 1);
        assert_eq!(start_location(&list, Some("gone"), Some(0)), 0);
        assert_eq!(start_location(&list, Some("demo"), None), 2);
        assert_eq!(start_location(&list, None, None), 0);
        assert_eq!(start_location(&[], None, Some(3)), 0);
    }

    #[test]
    fn the_location_tour_moves_on_every_so_often() {
        assert_eq!(LocationTour::from_autoplay("trick tour"), None);
        let mut t = LocationTour::from_autoplay("trick tour\nswitch 2\n").unwrap();
        let mut seen = vec![];
        let mut cur = 0;
        for _ in 0..(60 * 2 * 4) {
            if let Some(n) = t.next(cur, 3) {
                seen.push(n);
                cur = n;
            }
        }
        assert_eq!(seen, [1, 2, 0, 1]);
        // Location 1 fails to load: the game is on the demo park (2), and the tour goes on to 2, then 0.
        let mut t = LocationTour::every(1);
        assert_eq!(t.next(0, 3), Some(1));
        assert_eq!(t.next(2, 3), Some(2));
        assert_eq!(t.next(2, 3), Some(0));
        assert_eq!(LocationTour::every(5).next(0, 1), None);
    }

    #[test]
    fn a_long_list_scrolls_with_the_cursor() {
        assert_eq!(shown(5, 4), (0, 5));
        assert_eq!(shown(23, 0), (0, ROWS));
        assert_eq!(shown(23, 10), (4, 4 + ROWS));
        assert_eq!(shown(23, 22), (23 - ROWS, 23));
        for c in 0..23 {
            let (from, to) = shown(23, c);
            assert!(from <= c && c < to && to - from == ROWS);
        }
        // Drawn, it stays on the screen.
        let names: Vec<String> = (0..23).map(|i| format!("PLACE {i}")).collect();
        let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut menu = Menu::start(0, 0);
        menu.screen = Screen::Locations;
        for c in [0, 11, 22] {
            menu.cursor = c;
            let mut b = MeshBuilder::new();
            menu.draw(&mut b, &names, 3);
            assert!(
                b.verts
                    .iter()
                    .all(|v| v.pos[1] >= 0.0 && v.pos[1] <= SCREEN_H),
                "cursor {c}: drawn off the screen"
            );
        }
    }
}
