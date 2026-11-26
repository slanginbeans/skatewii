//! Riding a district that streams in: the game waits on the loading screen until the cells around the start
//! are read, then the rider rolls across cell edges while cells are read ahead and dropped behind, and the
//! rider is held (not dropped through the world) where the reads haven't caught up. Synthetic cells, stored
//! as the importer stores them (packed collision, meshes with the aligned header), drawn fogged.

use std::cell::RefCell;
use std::rc::Rc;

use game_core::Game;
use game_core::city::{CellInfo, Index};
use game_core::collision;
use game_core::district::District;
use game_core::gfx::{Camera, Fog, Gfx, StaticVerts, TexVertex, Texture, Vertex};
use game_core::input::{PadRaw, flag};
use game_core::math::Mat34;
use game_core::math::v3;
use game_core::pack::{self, Kind};
use game_core::reader::MemReader;

/// A cell pack: its 100 m square as a flat floor at y 0 (packed collision), and a quad drawn on it.
fn cell_pack(x: i32, z: i32) -> Vec<u8> {
    let (x0, x1) = (x as f32 - 50.0, x as f32 + 50.0);
    let (z0, z1) = (z as f32 - 50.0, z as f32 + 50.0);
    let floor = collision::encode_packed(&[
        ([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]], 1, 0),
        ([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]], 1, 0),
    ])
    .unwrap();
    let c = 0xffff_ffff;
    let mesh = game_core::world::encode_mesh(
        &[
            ([x0, 0.0, z0], c, [0.0, 0.0]),
            ([x0, 0.0, z1], c, [0.0, 1.0]),
            ([x1, 0.0, z1], c, [1.0, 1.0]),
            ([x1, 0.0, z0], c, [1.0, 0.0]),
        ],
        &[0, 1, 2, 0, 2, 3],
        None,
        0,
    );
    let spawn: Vec<u8> = [x as f32, 0.0, z as f32, 0.0]
        .iter()
        .flat_map(|f| f.to_be_bytes())
        .collect();
    pack::encode(&[
        ("park/collision", Kind::Collision, &floor, 0, 0, 0),
        ("park/spawn", Kind::Data, &spawn, 0, 0, 0),
        ("park/mesh/0", Kind::Data, &mesh, 0, 0, 0),
    ])
}

/// Counts the meshes drawn and keeps the fog asked for.
#[derive(Default)]
struct Seen {
    meshes: usize,
    fog: Option<Fog>,
}

impl Gfx for Seen {
    fn begin_frame(&mut self, _cam: &Camera, _clear: u32) {
        self.fog = None;
    }
    fn fog(&mut self, fog: Option<Fog>) {
        self.fog = fog;
    }
    fn draw_tris(&mut self, _v: &[Vertex], _m: &Mat34) {}
    fn draw_indexed(&mut self, _v: &[TexVertex], _i: &[u16], _t: Option<&Texture>, _m: &Mat34) {}
    fn draw_indexed_static(
        &mut self,
        v: &StaticVerts,
        i: &[u16],
        _t: Option<&Texture>,
        _m: &Mat34,
    ) {
        assert_eq!((v.len(), i.len()), (4, 6));
        self.meshes += 1;
    }
    fn draw_2d(&mut self, _v: &[Vertex]) {}
    fn end_frame(&mut self) {}
}

/// A 12 x 3 district of flat cells, the start near the -x end facing +x.
fn district() -> (District, Rc<RefCell<MemReader>>) {
    let reader = Rc::new(RefCell::new(MemReader::new()));
    let mut cells = Vec::new();
    for i in 0..12 {
        for j in -1..=1 {
            let (x, z) = (50 + 100 * i, 50 + 100 * j);
            let p = cell_pack(x, z);
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
                .add(&format!("data/city/Flat/c_{x}_{z}.fgpak"), p);
        }
    }
    let index = Index {
        cell: 100.0,
        floor: 0.0,
        starts: vec![(v3(20.0, 0.0, 50.0), core::f32::consts::FRAC_PI_2)],
        cells,
    };
    let d = District::new(
        &index.encode(),
        "data/city/Flat/",
        Box::new(reader.clone()),
        1 << 30,
    )
    .unwrap();
    (d, reader)
}

fn idle() -> PadRaw {
    PadRaw {
        flags: flag::CONNECTED,
        ..Default::default()
    }
}

#[test]
fn a_district_streams_in_while_the_rider_rolls_across_it() {
    let mut g = Game::new();
    let (d, r) = district();
    g.load_district(d);
    let dt = 1.0 / 60.0;

    // Nothing read: the loading screen, and the rider isn't placed yet.
    for _ in 0..10 {
        g.update(&idle(), dt);
    }
    assert!(g.district_loading());

    // Reads arrive (one per frame, like a slow card).
    let mut frames = 0;
    while g.district_loading() {
        r.borrow_mut().service();
        g.update(&idle(), dt);
        frames += 1;
        assert!(frames < 200, "the start never loaded");
    }
    let s = &g.skater;
    assert!(
        (s.pos - v3(20.0, 0.0, 50.0)).length() < 0.5,
        "at the start: {:?}",
        s.pos
    );
    assert!(
        g.log
            .iter()
            .any(|l| l.starts_with("district: cell 50 50 in"))
    );

    // Roll along +x at 8 m/s with a read done each frame: over several cell edges, never held, never off the
    // ground, cells dropped behind.
    g.place_rider(v3(20.0, 0.0, 50.0), core::f32::consts::FRAC_PI_2, 8.0);
    let mut low = f32::MAX;
    let mut frames_drawn = 0;
    for f in 0..60 * 120 {
        r.borrow_mut().service();
        g.update(&idle(), dt);
        low = low.min(g.skater.pos.y);
        if f % 60 == 0 {
            // Every second: the cells around are drawn (within the draw distance of the camera: the one
            // under the rider and its neighbours), fogged.
            let mut seen = Seen::default();
            g.render(&mut seen);
            assert!(
                (1..=12).contains(&seen.meshes),
                "cells drawn: {}",
                seen.meshes
            );
            assert!(
                seen.fog
                    .is_some_and(|f| f.end < game_core::world::DRAW_DISTANCE)
            );
            frames_drawn += 1;
        }
        if g.skater.speed < 7.0 {
            g.place_rider(g.skater.pos, core::f32::consts::FRAC_PI_2, 8.0);
        }
        if g.skater.pos.x > 900.0 {
            break;
        }
    }
    assert!(frames_drawn > 10);
    let s = &g.skater;
    assert!(s.pos.x > 900.0, "rode across the district: {:?}", s.pos);
    assert!(low > -0.1, "stayed on the ground (lowest {low})");
    assert!(s.rolling());
    let d = g.district().unwrap();
    assert!(
        d.loaded() <= 21,
        "cells behind dropped: {} loaded",
        d.loaded()
    );
    let live = r.borrow().live;
    assert!(
        live >= d.loaded() && live <= d.loaded() + d.busy(),
        "only loaded cells (and reads on their way) held: {live}"
    );
    assert!(
        g.log
            .iter()
            .any(|l| l.starts_with("district: cell 50 50 out"))
    );

    // The card stops answering: the rider rolls into cells that aren't read and is held at the edge.
    let mut last = g.skater.pos;
    let mut still = 0;
    for _ in 0..60 * 25 {
        g.update(&idle(), dt);
        if g.skater.speed < 7.0 {
            g.place_rider(g.skater.pos, core::f32::consts::FRAC_PI_2, 8.0);
        }
        let p = g.skater.pos;
        still = if (p - last).length() < 1e-6 {
            still + 1
        } else {
            0
        };
        last = p;
    }
    assert!(still > 60, "held where nothing is read");
    assert!(g.skater.pos.y > -0.1, "held on the ground, not dropped");
    assert!(!g.district().unwrap().ready(g.skater.pos));

    // Back to the demo park: every pack freed.
    g.unload_park();
    assert_eq!(r.borrow().live, 0);
}
