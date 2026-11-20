//! `fg_import build`: converts everything the game loads, as part of the game's build, and puts it in the app
//! folder. Each pack is converted only when it is missing or out of date, and copied only when the app
//! folder's copy differs.
//!
//! Up to date means its stamp (written next to it) matches: this converter's source hash, the disc image's
//! size and modification time, and the job's options. Without a disc image this does nothing and succeeds,
//! so the game builds with its placeholder art (CLAUDE.md rule 1).

use crate::xdvdfs::Disc;
use crate::{Res, park, skater};
use std::path::{Path, PathBuf};

/// The converter's source hash, from `build.rs`.
const SRC_HASH: &str = env!("FG_IMPORT_SRC_HASH");

/// One converted output: what the game loads from the app folder's `data/`.
struct Job {
    name: &'static str,
    /// Files the job writes into its output directory, all copied to the app folder.
    files: &'static [&'static str],
    args: Vec<String>,
}

/// The park the build converts unless `FG_PARK` names another (`fg_import park` lists them).
/// StartPark, the first district, is an empty warehouse; BlackBoxPark is a full skate park.
pub const DEFAULT_PARK: &str = "BlackBoxPark";

/// What the game loads.
fn jobs() -> Vec<Job> {
    let park = std::env::var("FG_PARK")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| DEFAULT_PARK.to_string());
    vec![
        Job {
            name: "skater",
            files: &["skater.fgpak"],
            // LOD 2: the Wii skins on its CPU every frame.
            args: vec!["--lod".into(), "2".into()],
        },
        Job {
            name: "park",
            files: &["park.fgpak"],
            args: vec!["--park".into(), park],
        },
    ]
}

const BUILD_USAGE: &str = "usage:
  fg_import build [options]       convert what the game loads and put it in the app folder
      --iso <path>        the disc image (default: $FG_ISO, then `iso=` in fg_local.txt, then the one
                          *.iso found in a --search directory or a folder inside it)
      --search <dir>      where to look for a disc image and fg_local.txt (repeatable)
      --cache <dir>       where converted packs and their stamps are kept (default: imported/build)
      --dest <dir>        the app folder's data directory to fill (optional)";

struct Options {
    iso: Option<PathBuf>,
    search: Vec<PathBuf>,
    cache: PathBuf,
    dest: Option<PathBuf>,
}

fn options(args: &[String]) -> Res<Options> {
    let mut o = Options {
        iso: None,
        search: Vec::new(),
        cache: PathBuf::from("imported/build"),
        dest: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().map(PathBuf::from).ok_or(BUILD_USAGE);
        match a.as_str() {
            "--iso" => o.iso = Some(val()?),
            "--search" => o.search.push(val()?),
            "--cache" => o.cache = val()?,
            "--dest" => o.dest = Some(val()?),
            "-h" | "--help" => return Err(BUILD_USAGE.into()),
            _ => return Err(format!("unknown option {a}\n{BUILD_USAGE}").into()),
        }
    }
    Ok(o)
}

/// `iso=<path>` from a `fg_local.txt` in one of the search directories (relative paths are taken from
/// that file's folder).
fn iso_from_local(search: &[PathBuf]) -> Option<PathBuf> {
    search.iter().find_map(|d| {
        let text = std::fs::read_to_string(d.join("fg_local.txt")).ok()?;
        text.lines().find_map(|l| {
            let v = l.trim().strip_prefix("iso=")?.trim().trim_matches('"');
            (!v.is_empty()).then(|| d.join(v))
        })
    })
}

/// Disc images in each search directory and the folders directly inside it.
fn find_isos(search: &[PathBuf]) -> Vec<PathBuf> {
    let is_iso =
        |p: &Path| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso"));
    let mut found = Vec::new();
    for d in search {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if is_iso(&p) {
                found.push(p);
            } else if p.is_dir()
                && let Ok(inner) = std::fs::read_dir(&p)
            {
                found.extend(inner.flatten().map(|e| e.path()).filter(|p| is_iso(p)));
            }
        }
    }
    // The same image reached through two search directories counts once; keep the path as found (Windows'
    // canonical form has a \\?\ prefix).
    let key = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
    found.sort_by_key(key);
    found.dedup_by(|a, b| key(a) == key(b));
    found
}

/// The disc image to convert from, or None (with the reason printed) to build with placeholder art.
fn pick_iso(o: &Options) -> Res<Option<PathBuf>> {
    if let Some(p) = &o.iso {
        return Ok(Some(p.clone()));
    }
    if let Some(p) = std::env::var_os("FG_ISO").filter(|v| !v.is_empty()) {
        return Ok(Some(PathBuf::from(p)));
    }
    if let Some(p) = iso_from_local(&o.search) {
        return Ok(Some(p));
    }
    let found = find_isos(&o.search);
    match found.len() {
        0 => {
            println!(
                "[assets] No Skate 3 disc image found, so the game uses its placeholder art.\n\
                 [assets] To import from your own disc: put the .iso next to this folder, or set FG_ISO,\n\
                 [assets] or write iso=<path> into fg_local.txt (docs/importing.md)."
            );
            Ok(None)
        }
        1 => Ok(found.into_iter().next()),
        _ => Err(format!(
            "found more than one disc image; pick one with FG_ISO or iso=<path> in fg_local.txt:\n  {}",
            found
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n  ")
        )
        .into()),
    }
}

/// What a job's output depends on. Equal stamps mean the files are up to date.
fn stamp(iso: &Path, job: &Job) -> Res<String> {
    let meta = std::fs::metadata(iso)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Ok(format!(
        "fg_import {} src {SRC_HASH}\ndisc {} bytes, modified {mtime}\njob {} {}\n",
        env!("CARGO_PKG_VERSION"),
        meta.len(),
        job.name,
        job.args.join(" ")
    ))
}

fn stamp_path(dir: &Path, job: &Job) -> PathBuf {
    dir.join(format!("{}.stamp", job.name))
}

/// True when `dir` holds every file of `job` and its stamp equals `want`.
fn up_to_date(dir: &Path, job: &Job, want: &str) -> bool {
    std::fs::read_to_string(stamp_path(dir, job)).is_ok_and(|s| s == want)
        && job.files.iter().all(|f| dir.join(f).is_file())
}

/// Copies `job`'s files and stamp from `cache` to `dest` unless `dest` already has the same version.
fn install(cache: &Path, dest: &Path, job: &Job, want: &str) -> Res<bool> {
    let same_size = |f: &&str| {
        let (a, b) = (cache.join(f), dest.join(f));
        matches!((a.metadata(), b.metadata()), (Ok(a), Ok(b)) if a.len() == b.len())
    };
    if up_to_date(dest, job, want) && job.files.iter().all(same_size) {
        return Ok(false);
    }
    std::fs::create_dir_all(dest)?;
    for f in job.files {
        std::fs::copy(cache.join(f), dest.join(f))?;
    }
    // The stamp last: an interrupted copy is redone next time.
    std::fs::write(stamp_path(dest, job), want)?;
    Ok(true)
}

fn convert(iso: &Path, job: &Job, out: &Path) -> Res<()> {
    let mut disc = Disc::open(iso)?;
    let mut args = job.args.clone();
    args.extend(["--out".to_string(), out.display().to_string()]);
    match job.name {
        "skater" => skater::run(&mut disc, &args),
        "park" => park::run(&mut disc, &args),
        other => Err(format!("unknown job {other}").into()),
    }
}

pub fn run(args: &[String]) -> Res<()> {
    let o = options(args)?;
    let Some(iso) = pick_iso(&o)? else {
        return Ok(());
    };
    if !iso.is_file() {
        return Err(format!("disc image {} not found", iso.display()).into());
    }
    println!("[assets] Disc image: {}", iso.display());
    for job in &jobs() {
        let want = stamp(&iso, job)?;
        if up_to_date(&o.cache, job, &want) {
            println!("[assets] {}: up to date", job.name);
        } else {
            println!("[assets] {}: converting...", job.name);
            std::fs::create_dir_all(&o.cache)?;
            // A failed conversion must not leave an old stamp claiming the files are current.
            let _ = std::fs::remove_file(stamp_path(&o.cache, job));
            convert(&iso, job, &o.cache).map_err(|e| format!("{}: {e}", job.name))?;
            std::fs::write(stamp_path(&o.cache, job), &want)?;
        }
        if let Some(dest) = &o.dest {
            if install(&o.cache, dest, job, &want)? {
                println!("[assets] {}: copied to {}", job.name, dest.display());
            } else {
                println!("[assets] {}: already in {}", job.name, dest.display());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fg_assets_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn job() -> Job {
        Job {
            name: "thing",
            files: &["a.fgpak", "b.fgpak"],
            args: vec!["--x".into()],
        }
    }

    #[test]
    fn stamps_decide_conversion_and_copying() {
        let job = job();
        let root = tmp("stamps");
        let iso = root.join("disc.iso");
        std::fs::write(&iso, b"not really a disc").unwrap();
        let (cache, dest) = (root.join("cache"), root.join("dest"));
        let want = stamp(&iso, &job).unwrap();
        assert!(want.contains(SRC_HASH) && want.contains("17 bytes") && want.contains("thing --x"));

        // Nothing there yet.
        assert!(!up_to_date(&cache, &job, &want));
        std::fs::create_dir_all(&cache).unwrap();
        for f in job.files {
            std::fs::write(cache.join(f), b"pack").unwrap();
        }
        // Files without a stamp are not trusted.
        assert!(!up_to_date(&cache, &job, &want));
        std::fs::write(stamp_path(&cache, &job), &want).unwrap();
        assert!(up_to_date(&cache, &job, &want));
        // A different disc or converter means a new stamp.
        assert!(!up_to_date(&cache, &job, "other"));

        // First install copies, the second doesn't.
        assert!(install(&cache, &dest, &job, &want).unwrap());
        assert!(!install(&cache, &dest, &job, &want).unwrap());
        // A missing or changed file in the app folder is copied again.
        std::fs::remove_file(dest.join("a.fgpak")).unwrap();
        assert!(install(&cache, &dest, &job, &want).unwrap());
        std::fs::write(dest.join("b.fgpak"), b"truncated?").unwrap();
        assert!(install(&cache, &dest, &job, &want).unwrap());
        assert_eq!(std::fs::read(dest.join("b.fgpak")).unwrap(), b"pack");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_the_disc_image() {
        let root = tmp("find");
        let o = |search: Vec<PathBuf>| Options {
            iso: None,
            search,
            cache: root.join("c"),
            dest: None,
        };
        // None: placeholder art, not an error.
        assert_eq!(pick_iso(&o(vec![root.clone()])).unwrap(), None);
        // One image, one folder down.
        std::fs::create_dir_all(root.join("Skate 3")).unwrap();
        std::fs::write(root.join("Skate 3").join("Skate 3.ISO"), b"x").unwrap();
        let found = pick_iso(&o(vec![root.clone()])).unwrap().unwrap();
        assert!(found.ends_with("Skate 3.ISO"));
        // Two: ambiguous, so it asks for a choice.
        std::fs::write(root.join("other.iso"), b"x").unwrap();
        assert!(pick_iso(&o(vec![root.clone()])).is_err());
        // fg_local.txt settles it.
        std::fs::write(root.join("fg_local.txt"), "# mine\niso=other.iso\n").unwrap();
        assert_eq!(
            pick_iso(&o(vec![root.clone()])).unwrap(),
            Some(root.join("other.iso"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
