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

/// The settings file in the app folder (`settings.txt`): `location <id>`, the last location chosen.
pub fn parse_settings(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        (w.next() == Some("location"))
            .then(|| w.next().map(|s| s.to_string()))
            .flatten()
    })
}

pub fn write_settings(location: &str) -> String {
    format!("# flatground settings\nlocation {location}\n")
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

pub const PAUSE_ITEMS: [&str; 4] = ["RESUME", "CHANGE LOCATION", "RESPAWN", "QUIT TO LOADER"];

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
        }
    }

    pub fn pause(held: u32) -> Menu {
        Menu {
            screen: Screen::Pause,
            cursor: 0,
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
                2 => Some(Choice::Respawn),
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
                PAUSE_ITEMS.iter().map(|s| s.to_string()).collect(),
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
        Some((current + 1) % count)
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
        let mut m = Menu::pause(btn::START);
        // The START that opened it is still held: nothing.
        assert_eq!(m.update(&pad(btn::START), DT, 4, 2), None);
        assert_eq!(tap(&mut m, btn::A, 4, 2), Some(Choice::Resume));
        let mut m = Menu::pause(0);
        assert_eq!(tap(&mut m, btn::START, 4, 2), Some(Choice::Resume));
        let mut m = Menu::pause(0);
        assert_eq!(tap(&mut m, btn::B, 4, 2), Some(Choice::Resume));
        // Change location: the list opens on the location being ridden; B comes back.
        let mut m = Menu::pause(0);
        tap(&mut m, btn::DPAD_DOWN, 4, 2);
        assert_eq!(tap(&mut m, btn::A, 4, 2), None);
        assert_eq!((m.screen, m.cursor), (Screen::Locations, 2));
        assert_eq!(tap(&mut m, btn::B, 4, 2), None);
        assert_eq!((m.screen, m.cursor), (Screen::Pause, 1));
        tap(&mut m, btn::A, 4, 2);
        tap(&mut m, btn::DPAD_UP, 4, 2);
        assert_eq!(tap(&mut m, btn::A, 4, 2), Some(Choice::Ride(1)));
        // Respawn and quit.
        let mut m = Menu::pause(0);
        tap(&mut m, btn::DPAD_DOWN, 4, 0);
        tap(&mut m, btn::DPAD_DOWN, 4, 0);
        assert_eq!(tap(&mut m, btn::A, 4, 0), Some(Choice::Respawn));
        let mut m = Menu::pause(0);
        tap(&mut m, btn::DPAD_UP, 4, 0);
        assert_eq!(tap(&mut m, btn::A, 4, 0), Some(Choice::Quit));
    }

    #[test]
    fn menus_draw_their_items() {
        let mut m = MeshBuilder::new();
        Menu::start(0, 0).draw(&mut m, &["Black Box Park", "Demo park"], 0);
        let start = m.verts.len();
        assert!(start > 0);
        let mut p = MeshBuilder::new();
        Menu::pause(0).draw(&mut p, &[], 0);
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
        let text = write_settings("MegaPark");
        assert_eq!(parse_settings(&text).as_deref(), Some("MegaPark"));
        assert_eq!(parse_settings("garbage\nlocation\n"), None);
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
