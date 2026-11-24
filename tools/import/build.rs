//! Hashes the converter's own sources into `FG_IMPORT_SRC_HASH`. `fg_import build` stamps every pack with it,
//! so packs are converted again after the converter changes, and only then.

use std::path::Path;

/// `game_core` modules that define pack formats. Add new ones here.
const FORMATS: &[&str] = &[
    "pack.rs",
    "collision.rs",
    "world.rs",
    "city.rs",
    "anim.rs",
    "skinned.rs",
    "tuning.rs",
    "gesture.rs",
    "tuning/grind.rs",
    "tuning/camera.rs",
    "tuning/moves.rs",
    "rails.rs",
    "graph/ops.rs",
    "graph/opcode.rs",
    "scoring/catalog.rs",
    "scoring/rules.rs",
    "sound/mod.rs",
    "sound/bank.rs",
    "sound/adpcm.rs",
];

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, files);
        } else if p.extension().is_some_and(|x| x == "rs") {
            files.push(p);
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The converter itself, plus the game's format definitions it writes (not the rest of the game, so
    // gameplay changes don't trigger a conversion).
    let src = root.join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut files = Vec::new();
    walk(&src, &mut files);
    for f in FORMATS {
        let p = root.join("../../game_core/src").join(f);
        println!("cargo:rerun-if-changed={}", p.display());
        files.push(p);
    }
    files.push(root.join("Cargo.toml"));
    files.sort();
    // FNV-1a 64 over each file's path (relative) and contents.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f);
        feed(rel.to_string_lossy().replace('\\', "/").as_bytes());
        // Line endings differ between checkouts (core.autocrlf); hash the text the same either way.
        let text = std::fs::read(f).unwrap_or_default();
        let text: Vec<u8> = text.into_iter().filter(|&b| b != b'\r').collect();
        feed(&text);
    }
    println!("cargo:rustc-env=FG_IMPORT_SRC_HASH={h:016x}");
}
