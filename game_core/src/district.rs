//! A district ridden while it streams: the cells of a [`city::Index`] read in around the rider in the
//! background ([`Reader`]) and dropped behind, within a memory budget.
//!
//! - Cells within [`LOAD_RADIUS`] of the rider are read, nearest first, a couple at a time, as long as the
//!   packs held stay within the budget (the cells the rider stands on and could reach soon,
//!   [`RIDE_RADIUS`], always). A finished read is parsed on the next call, one per call, so a frame never
//!   parses more than one cell.
//! - Cells beyond [`KEEP_RADIUS`] are dropped (and nearer ones, farthest first, when a nearer cell needs
//!   their room in the budget). Reads no longer wanted are cancelled.
//! - The game rides only where it is [`District::ready`]: every cell within [`RIDE_RADIUS`] of the rider
//!   is loaded (or missing from the card). Until then it holds the rider still.
//!
//! As ground the district is the union of its loaded cells: a triangle belongs to the cell holding its
//! centre but may reach into the next, so queries look at every loaded cell whose bounds they touch. Rails
//! keep their index while their cell is loaded (the grind state holds one): a dropped cell's rails are
//! replaced by inert ones far below the world, and their places are reused by cells loaded later.
//!
//! Dropping a cell frees its pack and its meshes, which the Wii's GPU reads in place: [`District::stream`]
//! must be called between frames, after the last frame's drawing finished (the game calls it from
//! `Game::update`; the Wii's frame ends waiting for the GPU).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

use crate::city::Index;
use crate::collision;
use crate::gfx::Gfx;
use crate::math::{Vec3, v3};
use crate::park::{Hit, Surface, Terrain};
use crate::rails::Rail;
use crate::reader::{Poll, Reader, Ticket};
use crate::world::{DRAW_DISTANCE, ImportedPark, MAX_DRAW_DISTANCE, MIN_DRAW_DISTANCE, Sight};

/// Cells within this (m, flat, to the cell's square) must be loaded to ride: what the rider stands on and
/// can reach in the next second or two.
pub const RIDE_RADIUS: f32 = 30.0;
/// Cells within this are read in: everything drawn, and some way beyond.
pub const LOAD_RADIUS: f32 = DRAW_DISTANCE + 20.0;
/// [`LOAD_RADIUS`] for the player's draw distance (the default's is [`LOAD_RADIUS`]).
pub fn load_radius(draw_distance: f32) -> f32 {
    draw_distance + 20.0
}
/// Loaded cells beyond this are dropped (the gap to [`LOAD_RADIUS`] keeps a cell on an edge from being read
/// and dropped over and over).
pub const KEEP_RADIUS: f32 = LOAD_RADIUS + 40.0;
/// Reads in flight at once.
pub const READS: usize = 2;

/// A dropped cell's rail: far below everything, so nothing finds it.
fn inert_rail() -> Rail {
    let far = v3(0.0, -1.0e6, 0.0);
    Rail::new(vec![far, far + v3(0.0, 0.0, 0.01)])
}

struct Cell {
    x: i32,
    z: i32,
    /// The pack, read in place (textures).
    pack: &'static [u8],
    park: ImportedPark,
    /// This cell's rails in [`District::rails`].
    rails: Range<usize>,
}

struct Read {
    x: i32,
    z: i32,
    ticket: Ticket,
}

pub struct District {
    pub index: Index,
    /// The district's folder, ending in `/` (`sd:/apps/flatground/data/city/DownTown/`).
    dir: String,
    reader: Box<dyn Reader>,
    /// Pack bytes held at once (loaded, waiting to be parsed and being read).
    pub budget: u32,
    cells: Vec<Cell>,
    reads: Vec<Read>,
    /// Read and waiting to be parsed.
    ready: Vec<(i32, i32, &'static [u8])>,
    /// Cells that couldn't be read or parsed: not tried again.
    failed: Vec<(i32, i32)>,
    rails: Vec<Rail>,
    /// Writes a parsed cell's vertex arrays out of the CPU cache (the Wii's GPU reads them in place).
    pub flush: Option<fn(*const u8, usize)>,
    /// Build each cell's draw commands for the GPU as it loads ([`ImportedPark::build_lists`]; the Wii).
    pub lists: bool,
    /// Microseconds, for the log's parse times.
    pub timer: Option<fn() -> u32>,
    /// What happened (cells loaded, dropped, failed), for the game's log.
    pub log: Vec<String>,
    /// Which of the index's starts the rider starts at (and respawns at).
    start: usize,
    kill: f32,
    /// How far the cells are drawn (the player's setting); what is read and kept follows it.
    draw_distance: f32,
}

impl District {
    /// A district from its index pack's `city/index` bytes, its folder and a reader. Nothing is read
    /// until [`District::stream`].
    pub fn new(
        index: &[u8],
        dir: &str,
        reader: Box<dyn Reader>,
        budget: u32,
    ) -> Result<District, collision::Error> {
        let index = Index::parse(index).ok_or(collision::Error::Bad)?;
        let kill = index.floor - 5.0;
        let mut dir = String::from(dir);
        if !dir.is_empty() && !dir.ends_with('/') {
            dir.push('/');
        }
        Ok(District {
            index,
            dir,
            reader,
            budget,
            cells: Vec::new(),
            reads: Vec::new(),
            ready: Vec::new(),
            failed: Vec::new(),
            rails: Vec::new(),
            flush: None,
            lists: false,
            timer: None,
            log: Vec::new(),
            start: 0,
            kill,
            draw_distance: DRAW_DISTANCE,
        })
    }

    /// Draws to `d` metres (clamped to the setting's range); cells are read and kept that much further out.
    pub fn set_draw_distance(&mut self, d: f32) {
        self.draw_distance = d.clamp(MIN_DRAW_DISTANCE, MAX_DRAW_DISTANCE);
    }

    pub fn draw_distance(&self) -> f32 {
        self.draw_distance
    }

    /// Starts the rider at the index's start `i` (the first if there is no such start).
    pub fn with_start(mut self, i: usize) -> District {
        self.start = if i < self.index.starts.len() { i } else { 0 };
        self
    }

    fn has(&self, x: i32, z: i32) -> bool {
        self.cells.iter().any(|c| (c.x, c.z) == (x, z))
    }
    fn reading(&self, x: i32, z: i32) -> bool {
        self.reads.iter().any(|r| (r.x, r.z) == (x, z))
            || self.ready.iter().any(|r| (r.0, r.1) == (x, z))
    }
    fn failed(&self, x: i32, z: i32) -> bool {
        self.failed.contains(&(x, z))
    }
    fn bytes_of(&self, x: i32, z: i32) -> u32 {
        self.index.find(x, z).map_or(0, |c| c.bytes)
    }
    /// Pack bytes held or on their way.
    pub fn held(&self) -> u32 {
        let loaded: u32 = self.cells.iter().map(|c| c.pack.len() as u32).sum();
        let ready: u32 = self.ready.iter().map(|r| r.2.len() as u32).sum();
        let reading: u32 = self.reads.iter().map(|r| self.bytes_of(r.x, r.z)).sum();
        loaded + ready + reading
    }

    /// Cells loaded now.
    pub fn loaded(&self) -> usize {
        self.cells.len()
    }

    /// Reads in flight or waiting to be parsed.
    pub fn busy(&self) -> usize {
        self.reads.len() + self.ready.len()
    }

    /// Is everything within [`RIDE_RADIUS`] of `at` loaded (or not there to load)?
    pub fn ready(&self, at: Vec3) -> bool {
        self.index
            .near(at.x, at.z, RIDE_RADIUS)
            .iter()
            .all(|(c, _)| self.has(c.x, c.z) || self.failed(c.x, c.z))
    }

    /// How many of the cells within [`RIDE_RADIUS`] of `at` are loaded, of how many (the loading screen).
    pub fn progress(&self, at: Vec3) -> (usize, usize) {
        let near = self.index.near(at.x, at.z, RIDE_RADIUS);
        let done = near
            .iter()
            .filter(|(c, _)| self.has(c.x, c.z) || self.failed(c.x, c.z))
            .count();
        (done, near.len())
    }

    /// Reads, parses and drops cells around `at`. Call once a frame, between frames (see the module).
    pub fn stream(&mut self, at: Vec3) {
        // Reads that ended.
        let mut i = 0;
        while i < self.reads.len() {
            let r = &self.reads[i];
            match self.reader.poll(r.ticket) {
                Poll::Waiting => i += 1,
                Poll::Done(data) => {
                    self.ready.push((r.x, r.z, data));
                    self.reads.swap_remove(i);
                }
                Poll::Failed => {
                    let (x, z) = (r.x, r.z);
                    self.log
                        .push(format!("district: cell {x} {z} could not be read"));
                    self.failed.push((x, z));
                    self.reads.swap_remove(i);
                }
            }
        }

        // Drop what is out of reach, and cancel reads no longer wanted.
        let keep: Vec<(i32, i32)> = self
            .index
            .near(
                at.x,
                at.z,
                load_radius(self.draw_distance) + (KEEP_RADIUS - LOAD_RADIUS),
            )
            .iter()
            .map(|(c, _)| (c.x, c.z))
            .collect();
        let mut k = 0;
        while k < self.cells.len() {
            if keep.contains(&(self.cells[k].x, self.cells[k].z)) {
                k += 1;
            } else {
                self.drop_cell(k);
            }
        }
        let reader = &mut self.reader;
        self.reads.retain(|r| {
            let wanted = keep.contains(&(r.x, r.z));
            if !wanted {
                reader.cancel(r.ticket);
            }
            wanted
        });
        let mut j = 0;
        while j < self.ready.len() {
            let (x, z, data) = self.ready[j];
            if keep.contains(&(x, z)) {
                j += 1;
            } else {
                // SAFETY: nothing was made from it.
                unsafe { self.reader.free(data) };
                self.ready.swap_remove(j);
            }
        }

        // Parse one finished read, the nearest.
        if let Some(n) = (0..self.ready.len()).min_by(|&a, &b| {
            let d = |i: usize| {
                let (x, z, _) = self.ready[i];
                let (dx, dz) = (x as f32 - at.x, z as f32 - at.z);
                dx * dx + dz * dz
            };
            d(a).total_cmp(&d(b))
        }) {
            let (x, z, data) = self.ready.swap_remove(n);
            self.add_cell(x, z, data);
        }

        // Start reads, nearest first.
        let want: Vec<(i32, i32, u32, f32)> = self
            .index
            .near(at.x, at.z, load_radius(self.draw_distance))
            .iter()
            .map(|(c, d)| (c.x, c.z, c.bytes, *d))
            .collect();
        for &(x, z, bytes, d) in &want {
            if self.reads.len() >= READS {
                break;
            }
            if self.has(x, z) || self.reading(x, z) || self.failed(x, z) {
                continue;
            }
            let must = d <= RIDE_RADIUS;
            // Make room by dropping loaded cells that are farther than this one, farthest first.
            while self.held() + bytes > self.budget {
                let far = self
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let (dx, dz) = (c.x as f32 - at.x, c.z as f32 - at.z);
                        (i, dx * dx + dz * dz)
                    })
                    .filter(|&(_, dd)| {
                        let (dx, dz) = (x as f32 - at.x, z as f32 - at.z);
                        dd > dx * dx + dz * dz
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                match far {
                    Some((i, _)) => self.drop_cell(i),
                    None => break,
                }
            }
            if self.held() + bytes > self.budget && !must {
                // Nearest first: nothing farther fits either if this doesn't... unless it's smaller.
                continue;
            }
            let path = format!("{}c_{x}_{z}.fgpak", self.dir);
            match self.reader.start(&path) {
                Some(ticket) => self.reads.push(Read { x, z, ticket }),
                None => break,
            }
        }
    }

    fn add_cell(&mut self, x: i32, z: i32, data: &'static [u8]) {
        let now = || self.timer.map_or(0, |f| f());
        let t0 = now();
        let mut park = match ImportedPark::parse(data) {
            Ok(p) => p,
            Err(e) => {
                self.log
                    .push(format!("district: cell {x} {z} unreadable ({e:?})"));
                self.failed.push((x, z));
                // SAFETY: the parse failed, nothing refers to it.
                unsafe { self.reader.free(data) };
                return;
            }
        };
        if let Some(flush) = self.flush {
            for m in &park.models {
                let (p, n) = m.verts.bytes();
                flush(p, n);
            }
        }
        if self.lists {
            park.build_lists(self.flush);
        }
        // Its rails go in the first run of inert rails long enough, else at the end.
        let rails = core::mem::take(&mut park.mesh.rails);
        let n = rails.len();
        let at = self.free_run(n);
        if at + n > self.rails.len() {
            self.rails.resize_with(at + n, inert_rail);
        }
        for (k, r) in rails.into_iter().enumerate() {
            self.rails[at + k] = r;
        }
        self.kill = self.kill.min(park.mesh.min.y - 5.0);
        let us = now().wrapping_sub(t0);
        self.log.push(format!(
            "district: cell {x} {z} in, {} KB, {} triangles, {n} rails, parsed in {} ms ({} cells, {} KB held)",
            data.len() / 1024,
            park.mesh.len(),
            us / 1000,
            self.cells.len() + 1,
            (self.held() as usize + data.len()) / 1024
        ));
        self.cells.push(Cell {
            x,
            z,
            pack: data,
            park,
            rails: at..at + n,
        });
    }

    /// Where `n` rails fit: the first gap between loaded cells' rails that is long enough, else the end.
    fn free_run(&self, n: usize) -> usize {
        let mut used: Vec<Range<usize>> = self
            .cells
            .iter()
            .map(|c| c.rails.clone())
            .filter(|r| !r.is_empty())
            .collect();
        used.sort_by_key(|r| r.start);
        let mut from = 0;
        for r in &used {
            if r.start - from >= n {
                return from;
            }
            from = r.end;
        }
        if n == 0 { 0 } else { from }
    }

    fn drop_cell(&mut self, i: usize) {
        let c = self.cells.swap_remove(i);
        for r in c.rails.clone() {
            self.rails[r] = inert_rail();
        }
        self.log.push(format!(
            "district: cell {} {} out ({} cells left)",
            c.x,
            c.z,
            self.cells.len()
        ));
        let pack = c.pack;
        drop(c.park);
        // SAFETY: its park (meshes, textures) is gone and the last frame finished drawing (see the module).
        unsafe { self.reader.free(pack) };
    }

    /// Draws the loaded cells' models (or collision) within the draw distance of `eye`: meshes and
    /// vertices drawn.
    pub fn draw(&self, gfx: &mut dyn Gfx, sight: &Sight) -> (u32, u32) {
        let eye = sight.eye;
        let h = self.index.cell * 0.5;
        let reach = sight.reach + self.index.cell;
        let (mut draws, mut verts) = (0, 0);
        for c in &self.cells {
            let dx = (libm::fabsf(eye.x - c.x as f32) - h).max(0.0);
            let dz = (libm::fabsf(eye.z - c.z as f32) - h).max(0.0);
            if dx * dx + dz * dz > reach * reach {
                continue;
            }
            let (d, v) = c.park.draw(gfx, sight);
            draws += d;
            verts += v;
        }
        (draws, verts)
    }

    /// Loaded cells' parks (for the log and the Wii's flush).
    pub fn parks(&self) -> impl Iterator<Item = &ImportedPark> {
        self.cells.iter().map(|c| &c.park)
    }

    /// Loaded cells whose collision's bounds (x, z) come within `m` of the box `lo..hi`.
    fn touching(&self, lo: Vec3, hi: Vec3, m: f32) -> impl Iterator<Item = &collision::TriMesh> {
        self.cells.iter().map(|c| &c.park.mesh).filter(move |t| {
            lo.x <= t.max.x + m && hi.x >= t.min.x - m && lo.z <= t.max.z + m && hi.z >= t.min.z - m
        })
    }
}

fn span(a: Vec3, b: Vec3) -> (Vec3, Vec3) {
    (
        v3(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
        v3(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
    )
}

impl Terrain for District {
    fn ground(&self, x: f32, z: f32, max: f32) -> Surface {
        let p = v3(x, 0.0, z);
        let mut best = Surface {
            height: f32::MIN,
            normal: Vec3::UP,
        };
        for t in self.touching(p, p, 0.0) {
            let s = t.ground(x, z, max);
            if s.height > best.height {
                best = s;
            }
        }
        best
    }
    fn blocked(&self, a: Vec3, b: Vec3) -> bool {
        let (lo, hi) = span(a, b);
        self.touching(lo, hi, 0.0).any(|t| t.blocked(a, b))
    }
    fn occluded(&self, p: Vec3, q: Vec3) -> bool {
        let (lo, hi) = span(p, q);
        self.touching(lo, hi, 0.0).any(|t| t.occluded(p, q))
    }
    fn raycast(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let (lo, hi) = span(from, from + dir * max);
        self.touching(lo, hi, 0.0)
            .filter_map(|t| t.raycast(from, dir, max))
            .min_by(|a, b| a.t.total_cmp(&b.t))
    }
    fn spawn(&self) -> (Vec3, f32) {
        self.index.starts[self.start]
    }
    fn kill_height(&self) -> f32 {
        self.kill
    }
    fn rails(&self) -> &[Rail] {
        &self.rails
    }
    fn triangles_near(&self, c: Vec3, r: f32, out: &mut Vec<([Vec3; 3], Vec3)>) -> bool {
        let m = v3(r, r, r);
        for t in self.touching(c - m, c + m, 0.0) {
            t.triangles_near(c, r, out);
        }
        true
    }
}

impl Drop for District {
    /// Frees every pack and cancels the reads. Like dropping a cell, only between frames.
    fn drop(&mut self) {
        for r in self.reads.drain(..) {
            self.reader.cancel(r.ticket);
        }
        for (_, _, d) in core::mem::take(&mut self.ready) {
            unsafe { self.reader.free(d) };
        }
        while !self.cells.is_empty() {
            let c = self.cells.swap_remove(0);
            let pack = c.pack;
            drop(c.park);
            unsafe { self.reader.free(pack) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::CellInfo;
    use crate::pack::{self, Kind};
    use crate::rails;
    use crate::reader::MemReader;
    use alloc::rc::Rc;
    use core::cell::RefCell;

    /// A cell pack: its square as a floor at height `y`, and one rail along its middle.
    fn cell_pack(x: i32, z: i32, y: f32) -> Vec<u8> {
        let (x0, x1) = (x as f32 - 50.0, x as f32 + 50.0);
        let (z0, z1) = (z as f32 - 50.0, z as f32 + 50.0);
        let floor = collision::encode(&[
            ([[x0, y, z0], [x0, y, z1], [x1, y, z1]], 1, 0),
            ([[x0, y, z0], [x1, y, z1], [x1, y, z0]], 1, 0),
        ]);
        let spawn: Vec<u8> = [x as f32, y, z as f32, 0.0]
            .iter()
            .flat_map(|f| f.to_be_bytes())
            .collect();
        let rail = rails::encode(&[(
            vec![
                [x as f32 - 5.0, y + 0.5, z as f32],
                [x as f32 + 5.0, y + 0.5, z as f32],
            ],
            vec![],
        )]);
        pack::encode(&[
            ("park/collision", Kind::Collision, &floor, 0, 0, 0),
            ("park/spawn", Kind::Data, &spawn, 0, 0, 0),
            ("park/rails", Kind::Data, &rail, 0, 0, 0),
        ])
    }

    /// Cells in a row along +x from (50, 50), each a floor 1 m higher than the last, and their reader.
    fn row(n: i32) -> (District, Rc<RefCell<MemReader>>) {
        let reader = Rc::new(RefCell::new(MemReader::new()));
        let mut cells = Vec::new();
        for i in 0..n {
            let (x, z) = (50 + 100 * i, 50);
            let p = cell_pack(x, z, i as f32);
            cells.push(CellInfo {
                x,
                z,
                bytes: p.len() as u32,
                tris: 2,
                vertices: 0,
                texture_bytes: 0,
            });
            reader
                .borrow_mut()
                .add(&format!("city/Row/c_{x}_{z}.fgpak"), p);
        }
        let index = Index {
            cell: 100.0,
            floor: 0.0,
            starts: vec![(v3(10.0, 0.0, 50.0), 0.0)],
            cells,
        };
        let d = District::new(
            &index.encode(),
            "city/Row",
            Box::new(reader.clone()),
            1 << 30,
        )
        .unwrap();
        (d, reader)
    }

    /// Streams at `at` until nothing more is read.
    fn settle(d: &mut District, r: &Rc<RefCell<MemReader>>, at: Vec3) {
        for _ in 0..100 {
            d.stream(at);
            let did = r.borrow_mut().service();
            if !did && d.busy() == 0 {
                d.stream(at);
                if d.busy() == 0 {
                    return;
                }
            }
        }
        panic!("never settled");
    }

    fn loaded(d: &District) -> Vec<i32> {
        let mut v: Vec<i32> = d.cells.iter().map(|c| c.x).collect();
        v.sort();
        v
    }

    #[test]
    fn cells_stream_in_around_the_rider_and_out_behind() {
        let (mut d, r) = row(10);
        let start = d.spawn().0;
        assert!(!d.ready(start), "nothing read yet");
        assert_eq!(d.ground(10.0, 50.0, 5.0).height, f32::MIN);
        d.stream(start);
        assert_eq!(d.busy(), READS, "two reads at a time");
        settle(&mut d, &r, start);
        assert!(d.ready(start));
        // Within the load radius of x 10: the cells at 50 and 150.
        assert_eq!(loaded(&d), [50, 150]);
        assert_eq!(d.ground(10.0, 50.0, 5.0).height, 0.0);
        assert_eq!(
            d.ground(140.0, 50.0, 5.0).height,
            1.0,
            "the next cell's floor"
        );
        assert_eq!(r.borrow().live, 2);

        // Ride to x 500: the first two are dropped, everything near read.
        let there = v3(500.0, 4.0, 50.0);
        assert!(!d.ready(there));
        settle(&mut d, &r, there);
        assert!(d.ready(there));
        assert_eq!(loaded(&d), [350, 450, 550, 650]);
        assert_eq!(r.borrow().live, 4, "dropped cells' packs freed");
        assert_eq!(d.ground(10.0, 50.0, 5.0).height, f32::MIN);
        assert_eq!(d.ground(500.0, 50.0, 10.0).height, 5.0);
        assert!(d.log.iter().any(|l| l.contains("cell 50 50 out")));

        drop(d);
        assert_eq!(r.borrow().live, 0, "everything freed with the district");
    }

    #[test]
    fn a_rail_keeps_its_index_while_its_cell_is_loaded() {
        let (mut d, r) = row(10);
        let at = v3(150.0, 1.0, 50.0);
        settle(&mut d, &r, at);
        let i = d
            .rails()
            .iter()
            .position(|rl| rl.near(v3(150.0, 1.5, 50.0), 0.1))
            .unwrap();
        // Moving on drops cells behind and loads more ahead; the rail at 150 stays where it was.
        for x in [200.0, 250.0, 290.0] {
            settle(&mut d, &r, v3(x, 1.0, 50.0));
            assert!(d.rails()[i].near(v3(150.0, 1.5, 50.0), 0.1), "at x {x}");
        }
        // Once its cell is dropped the rail is inert, and its place is reused.
        settle(&mut d, &r, v3(700.0, 1.0, 50.0));
        assert!(!d.rails()[i].near(v3(150.0, 1.5, 50.0), 0.1));
        let live = d.rails().iter().filter(|rl| rl.max.y > -1000.0).count();
        assert_eq!(live, d.loaded(), "one rail per loaded cell");
        assert!(d.rails().len() <= 8, "places reused: {}", d.rails().len());
    }

    #[test]
    fn the_budget_holds_and_the_nearest_cells_win() {
        let (mut d, r) = row(10);
        let one = d.index.cells[0].bytes;
        d.budget = one * 2 + one / 2;
        let at = v3(500.0, 4.0, 50.0);
        settle(&mut d, &r, at);
        assert_eq!(loaded(&d), [450, 550], "two fit; the ones the rider is on");
        assert!(d.held() <= d.budget);
        assert!(d.ready(at));
        // A missing cell is given up on, not waited for.
        let (mut d, r) = row(3);
        r.borrow_mut().add("unused", vec![]);
        d.index.cells.push(CellInfo {
            x: 350,
            z: 50,
            bytes: 10,
            tris: 0,
            vertices: 0,
            texture_bytes: 0,
        });
        let at = v3(300.0, 2.0, 50.0);
        settle(&mut d, &r, at);
        assert!(d.ready(at));
        assert!(
            d.log
                .iter()
                .any(|l| l.contains("cell 350 50 could not be read"))
        );
    }

    #[test]
    fn ground_and_rays_reach_across_cells() {
        let (mut d, r) = row(3);
        settle(&mut d, &r, v3(100.0, 0.0, 50.0));
        // A ray straight down onto each side of the edge at x 100.
        let down = v3(0.0, -1.0, 0.0);
        let hit = d.raycast(v3(99.0, 5.0, 50.0), down, 10.0).unwrap();
        assert!((hit.point.y - 0.0).abs() < 1e-4);
        let hit = d.raycast(v3(101.0, 5.0, 50.0), down, 10.0).unwrap();
        assert!((hit.point.y - 1.0).abs() < 1e-4);
        assert_eq!(
            d.ground(101.0, 50.0, 0.5).height,
            f32::MIN,
            "only the 1 m floor there"
        );
        assert_eq!(d.ground(99.0, 50.0, 0.5).height, 0.0);
        assert!(d.kill_height() < -4.0);
    }

    #[test]
    fn a_chosen_start_and_the_floor() {
        let (d, _) = row(3);
        assert_eq!(d.spawn(), (v3(10.0, 0.0, 50.0), 0.0));
        assert_eq!(d.kill_height(), -5.0, "below the district's lowest point");
        let mut index = d.index.clone();
        index.starts.push((v3(250.0, 3.0, 50.0), 1.0));
        let make = |i| {
            District::new(&index.encode(), "x", Box::new(MemReader::new()), 1 << 20)
                .unwrap()
                .with_start(i)
        };
        assert_eq!(make(1).spawn(), (v3(250.0, 3.0, 50.0), 1.0));
        assert_eq!(
            make(7).spawn().0,
            v3(10.0, 0.0, 50.0),
            "no such start: the first"
        );
    }
}
