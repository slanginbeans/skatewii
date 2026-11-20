//! Controller input. The platform fills a [`PadRaw`] (same layout as `FgPadRaw` in
//! `wii/platform/fg_platform.h`); the game reads a normalised [`Pad`].

/// Raw pad state as the platform layer reports it. Keep in sync with `FgPadRaw` in the C header.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadRaw {
    /// Main stick, about -100..100 on a GameCube pad (+Y is up).
    pub stick_x: i8,
    pub stick_y: i8,
    /// C-stick, about -80..80.
    pub cstick_x: i8,
    pub cstick_y: i8,
    /// Analog triggers 0..255.
    pub trig_l: u8,
    pub trig_r: u8,
    pub _pad: u16,
    /// `btn::*` bits.
    pub buttons: u32,
    /// `flag::*` bits.
    pub flags: u32,
}

/// Button bits used in [`PadRaw::buttons`]. The platform maps its own button codes onto these.
pub mod btn {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const X: u32 = 1 << 2;
    pub const Y: u32 = 1 << 3;
    pub const Z: u32 = 1 << 4;
    pub const START: u32 = 1 << 5;
    pub const L: u32 = 1 << 6;
    pub const R: u32 = 1 << 7;
    pub const DPAD_UP: u32 = 1 << 8;
    pub const DPAD_DOWN: u32 = 1 << 9;
    pub const DPAD_LEFT: u32 = 1 << 10;
    pub const DPAD_RIGHT: u32 = 1 << 11;
}

/// Bits in [`PadRaw::flags`].
pub mod flag {
    /// A controller is plugged into port 1.
    pub const CONNECTED: u32 = 1 << 0;
    /// The player asked to quit to the loader (HOME on a Wii Remote, or START + Z on the pad).
    pub const EXIT: u32 = 1 << 1;
}

/// Normalised input for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    /// Left stick, each axis -1..1, radial dead zone applied. +Y is up (away from the player).
    pub stick: [f32; 2],
    /// Right stick (the C-stick on a GameCube pad): the flick stick.
    pub flick: [f32; 2],
    pub trig_l: f32,
    pub trig_r: f32,
    pub held: u32,
    /// Buttons that went down this frame.
    pub pressed: u32,
}

/// Raw stick values at full deflection, measured on a GameCube pad (session logs: the main stick reached
/// +-103, the C-stick +-97). Skate 3's conditioning then counts 95% of this as fully over.
pub const MAIN_RANGE: f32 = 100.0;
pub const C_RANGE: f32 = 92.0;
/// Skate 3's stick conditioning: a radial dead zone, then full deflection a little before the rim.
const DEAD_ZONE: f32 = 0.25;
const FULL: f32 = 0.95;

fn stick(x: i8, y: i8, range: f32) -> [f32; 2] {
    let (x, y) = (x as f32 / range, y as f32 / range);
    let mag = libm::sqrtf(x * x + y * y);
    if mag < DEAD_ZONE {
        return [0.0, 0.0];
    }
    // Rescale so the output starts at 0 just outside the dead zone and saturates at 1.
    let scaled = ((mag - DEAD_ZONE) / (FULL - DEAD_ZONE)).min(1.0);
    [x / mag * scaled, y / mag * scaled]
}

impl Pad {
    pub fn from_raw(raw: &PadRaw, previous_held: u32) -> Pad {
        Pad {
            stick: stick(raw.stick_x, raw.stick_y, MAIN_RANGE),
            flick: stick(raw.cstick_x, raw.cstick_y, C_RANGE),
            trig_l: raw.trig_l as f32 / 255.0,
            trig_r: raw.trig_r as f32 / 255.0,
            held: raw.buttons,
            pressed: raw.buttons & !previous_held,
        }
    }
    pub fn is_held(&self, b: u32) -> bool {
        self.held & b != 0
    }
    pub fn was_pressed(&self, b: u32) -> bool {
        self.pressed & b != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_layout_matches_c_header() {
        // FgPadRaw: 4 x s8, 2 x u8, u16, u32, u32 = 16 bytes, buttons at 8, flags at 12.
        assert_eq!(core::mem::size_of::<PadRaw>(), 16);
        assert_eq!(core::mem::offset_of!(PadRaw, buttons), 8);
        assert_eq!(core::mem::offset_of!(PadRaw, flags), 12);
    }

    #[test]
    fn dead_zone_and_saturation() {
        let r = PadRaw {
            stick_x: 12,
            stick_y: -12,
            cstick_x: 100,
            ..Default::default()
        };
        let p = Pad::from_raw(&r, 0);
        assert_eq!(p.stick, [0.0, 0.0]);
        assert!((p.flick[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn pressed_is_edge_triggered() {
        let r = PadRaw {
            buttons: btn::A | btn::B,
            ..Default::default()
        };
        let p = Pad::from_raw(&r, btn::A);
        assert!(p.is_held(btn::A) && !p.was_pressed(btn::A));
        assert!(p.was_pressed(btn::B));
    }
}
