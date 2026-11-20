//! Low chase camera that trails the direction of travel.

use crate::gfx::Camera;
use crate::math::{Vec3, smooth, v3, yaw_dir};
use crate::park::Terrain;
use crate::skater::{Mode, Skater};

const DISTANCE: f32 = 3.6;
const HEIGHT: f32 = 1.5;
const LOOK_HEIGHT: f32 = 0.8;

pub struct ChaseCam {
    eye: Vec3,
    target: Vec3,
}

impl ChaseCam {
    pub fn new(s: &Skater) -> ChaseCam {
        let (eye, target) = Self::wanted(s);
        ChaseCam { eye, target }
    }

    fn wanted(s: &Skater) -> (Vec3, Vec3) {
        let travel = match s.mode {
            Mode::Air { .. } | Mode::Bailed { .. } if s.vel.horizontal().length() > 0.5 => {
                s.vel.horizontal().normalize_or(yaw_dir(s.heading))
            }
            _ => yaw_dir(s.heading),
        };
        let target = s.pos + v3(0.0, LOOK_HEIGHT, 0.0);
        (s.pos - travel * DISTANCE + v3(0.0, HEIGHT, 0.0), target)
    }

    pub fn update(&mut self, s: &Skater, park: &dyn Terrain, dt: f32) {
        let (eye, target) = Self::wanted(s);
        // Follow height more loosely in the air so jumps read as jumps.
        let k = smooth(if s.is_airborne() { 3.0 } else { 6.0 }, dt);
        self.eye = self.eye.lerp(eye, k);
        self.target = self.target.lerp(target, smooth(12.0, dt));
        // Keep the lens out of the ground and ramps.
        let floor = park.ground(self.eye.x, self.eye.z, self.eye.y + 1.0).height + 0.4;
        self.eye.y = self.eye.y.max(floor);
        // A wall between the skater and the lens: move in until the skater is in view.
        if park.occluded(self.target, self.eye) {
            let back = self.eye - self.target;
            if let Some(k) = [0.8, 0.6, 0.4, 0.25]
                .into_iter()
                .find(|k| !park.occluded(self.target, self.target + back * *k))
            {
                self.eye = self.target + back * k;
            } else {
                self.eye = self.target + back * 0.15;
            }
        }
    }

    pub fn camera(&self) -> Camera {
        Camera {
            eye: self.eye,
            target: self.target,
            fov_y_deg: 58.0,
            near: 0.1,
            far: 120.0,
        }
    }
}
