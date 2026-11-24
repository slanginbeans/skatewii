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
    /// Which converter runs it (`skater`, `motion`, `rules`, `sound`, `park`).
    kind: &'static str,
    /// The job's stamp is `<name>.stamp` (a park's sits next to its pack: `parks/<park>.stamp`).
    name: String,
    /// Files the job writes into its output directory, all copied to the app folder.
    files: Vec<String>,
    /// A job writing a folder of files it decides itself (a district's cells): the folder, copied whole.
    dir: Option<String>,
    args: Vec<String>,
}

/// The location the build makes the default (first in the menu, ridden at first start) unless `FG_PARK`
/// names another (`fg_import park` lists them). StartPark, the first district, is an empty warehouse;
/// BlackBoxPark is a full skate park.
pub const DEFAULT_PARK: &str = "BlackBoxPark";

/// The location index the game reads (`data/parks.txt`).
const INDEX: &str = "parks.txt";

/// The default location: `FG_PARK` if it names a park, else [`DEFAULT_PARK`].
fn default_park() -> Res<&'static str> {
    match std::env::var("FG_PARK").ok().filter(|p| !p.is_empty()) {
        Some(p) => park::canonical(&p).ok_or_else(|| {
            format!(
                "FG_PARK={p} is not a park; parks: {}",
                park::PARKS.join(", ")
            )
            .into()
        }),
        None => Ok(DEFAULT_PARK),
    }
}

/// The locations the build converts, the default first (it is converted even when it is not one of
/// `park::LOCATIONS`: asked for by name).
fn locations(default: &'static str) -> Vec<&'static str> {
    let mut list = vec![default];
    list.extend(park::LOCATIONS.iter().filter(|p| **p != default));
    list
}

fn park_job(name: &str) -> Job {
    let file = format!("parks/{name}.fgpak");
    Job {
        kind: "park",
        name: format!("parks/{name}"),
        files: vec![file.clone()],
        dir: None,
        args: vec!["--park".into(), name.into(), "--file".into(), file],
    }
}

/// A streamed district: `city/<district>/` (its index and cell packs), stamped `city/<district>.stamp`.
fn district_job(name: &str) -> Job {
    Job {
        kind: "district",
        name: format!("city/{name}"),
        files: vec![format!("city/{name}/index.fgpak")],
        dir: Some(format!("city/{name}")),
        args: vec!["--district".into(), name.into()],
    }
}

/// The streamed districts are converted unless `FG_CITY=0` (they are about 330 MB).
fn with_districts() -> bool {
    std::env::var("FG_CITY").as_deref() != Ok("0")
}

/// What the game loads.
fn jobs(default: &'static str) -> Vec<Job> {
    let one = |kind: &'static str, args: Vec<String>| Job {
        kind,
        name: kind.into(),
        files: vec![format!("{kind}.fgpak")],
        dir: None,
        args,
    };
    let mut jobs = vec![
        // LOD 2: the Wii skins on its CPU every frame.
        one("skater", vec!["--lod".into(), "2".into()]),
        one("motion", Vec::new()),
        one("rules", Vec::new()),
        one("sound", Vec::new()),
    ];
    jobs.extend(locations(default).into_iter().map(park_job));
    if with_districts() {
        jobs.extend(crate::city::DISTRICTS.iter().map(|d| district_job(d)));
    }
    jobs
}

/// The location index for the converted parks in `dir` (sizes from the packs there).
fn index(dir: &Path, default: &'static str) -> String {
    let list: Vec<_> = locations(default)
        .into_iter()
        .map(|p| {
            let file = format!("parks/{p}.fgpak");
            let bytes = std::fs::metadata(dir.join(&file)).map_or(0, |m| m.len() as u32);
            game_core::menu::Location {
                id: p.into(),
                name: park::display_name(p),
                file,
                bytes,
                district: None,
            }
        })
        .chain(crate::city::DISTRICTS.iter().flat_map(|d| {
            // Converted districts (unless `FG_CITY=0`): the index and every cell next to it, one location per
            // start (`DownTown`, `DownTown-2`, ...).
            let file = format!("city/{d}/index.fgpak");
            let starts = std::fs::read(dir.join(&file)).ok().and_then(|b| {
                let pack = game_core::pack::Pack::parse(&b).ok()?;
                let index = game_core::city::Index::parse(pack.find("city/index")?.data)?;
                Some(index.starts.len().min(u8::MAX as usize))
            });
            let bytes = std::fs::read_dir(dir.join("city").join(d))
                .map(|it| {
                    it.filter_map(|e| e.ok()?.metadata().ok())
                        .map(|m| m.len())
                        .sum::<u64>()
                })
                .unwrap_or(0);
            (0..starts.unwrap_or(0)).map(move |s| {
                let (id, name) = match s {
                    0 => ((*d).to_string(), park::display_name(d)),
                    _ => (
                        format!("{d}-{}", s + 1),
                        format!("{} {}", park::display_name(d), s + 1),
                    ),
                };
                game_core::menu::Location {
                    id,
                    name,
                    file: file.clone(),
                    bytes: bytes.min(u32::MAX as u64) as u32,
                    district: Some(s as u8),
                }
            })
        }))
        .collect();
    game_core::menu::write_index(&list, Some(default))
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
    // Inside the job's own folder (`parks/`) when its name has one.
    dir.join(format!("{}.stamp", job.name))
}

/// True when `dir` holds every file of `job` and its stamp equals `want`.
fn up_to_date(dir: &Path, job: &Job, want: &str) -> bool {
    std::fs::read_to_string(stamp_path(dir, job)).is_ok_and(|s| s == want)
        && job.files.iter().all(|f| dir.join(f).is_file())
}

/// Every file `job` wrote into `dir`: its own list, or everything in its folder.
fn job_files(dir: &Path, job: &Job) -> Res<Vec<String>> {
    let Some(sub) = &job.dir else {
        return Ok(job.files.clone());
    };
    let mut files = Vec::new();
    for e in std::fs::read_dir(dir.join(sub))?.flatten() {
        if e.path().is_file() {
            files.push(format!("{sub}/{}", e.file_name().to_string_lossy()));
        }
    }
    files.sort();
    Ok(files)
}

/// Copies `job`'s files and stamp from `cache` to `dest` unless `dest` already has the same version. A
/// folder job's files that the cache no longer has are removed from `dest`.
fn install(cache: &Path, dest: &Path, job: &Job, want: &str) -> Res<bool> {
    let same_size = |f: &&str| {
        let (a, b) = (cache.join(f), dest.join(f));
        matches!((a.metadata(), b.metadata()), (Ok(a), Ok(b)) if a.len() == b.len())
    };
    let files = job_files(cache, job)?;
    if up_to_date(dest, job, want)
        && files.iter().map(String::as_str).all(|f| same_size(&f))
        && (job.dir.is_none() || job_files(dest, job).is_ok_and(|d| d == files))
    {
        return Ok(false);
    }
    if let Some(sub) = &job.dir {
        // The old version's stamp first: an interrupted copy is redone next time.
        let _ = std::fs::remove_file(stamp_path(dest, job));
        if dest.join(sub).is_dir() {
            for f in job_files(dest, job)? {
                if !files.contains(&f) {
                    std::fs::remove_file(dest.join(f))?;
                }
            }
        }
    }
    for f in &files {
        let to = dest.join(f);
        std::fs::create_dir_all(to.parent().unwrap_or(dest))?;
        std::fs::copy(cache.join(f), to)?;
    }
    // The stamp last: an interrupted copy is redone next time.
    std::fs::write(stamp_path(dest, job), want)?;
    Ok(true)
}

fn convert(iso: &Path, job: &Job, out: &Path) -> Res<()> {
    let mut disc = Disc::open(iso)?;
    let mut args = job.args.clone();
    args.extend(["--out".to_string(), out.display().to_string()]);
    match job.kind {
        "skater" => skater::run(&mut disc, &args),
        "park" => park::run(&mut disc, &args),
        "rules" => crate::rules::run(&mut disc, &args),
        "sound" => crate::sound::run(&mut disc, &args),
        "motion" => crate::motion::run(&mut disc, &args),
        "district" => {
            let mut args = job.args.clone();
            args.extend(["--out".to_string(), out.join("city").display().to_string()]);
            crate::district::run(&mut disc, &args)
        }
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
    let default = default_park()?;
    // Sound needs FFmpeg's XMA2 decoder (Skate 3's audio is Xbox 360 XMA); without one the game keeps its
    // placeholder sounds.
    let can_sound = crate::sound::decoder_available();
    for job in &jobs(default) {
        if job.kind == "sound" && !can_sound {
            println!(
                "[assets] sound: skipped, no FFmpeg with an XMA2 decoder found (install FFmpeg or set FG_FFMPEG;
                 [assets] docs/importing.md); the game plays its placeholder sounds"
            );
            continue;
        }
        let want = stamp(&iso, job)?;
        if up_to_date(&o.cache, job, &want) {
            println!("[assets] {}: up to date", job.name);
        } else {
            println!("[assets] {}: converting...", job.name);
            std::fs::create_dir_all(o.cache.join(&job.name).parent().unwrap_or(&o.cache))?;
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
    // The location index, next to the parks.
    let text = index(&o.cache, default);
    std::fs::write(o.cache.join(INDEX), &text)?;
    if let Some(dest) = &o.dest {
        if std::fs::read_to_string(dest.join(INDEX)).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(dest.join(INDEX), &text)?;
            println!("[assets] {INDEX}: written to {}", dest.display());
        }
        // Builds before the location index made one park.fgpak; the game reads parks.txt now.
        for old in ["park.fgpak", "park.stamp"] {
            if std::fs::remove_file(dest.join(old)).is_ok() {
                println!("[assets] removed the old {old} from {}", dest.display());
            }
        }
    }
    let total: u64 = locations(default)
        .iter()
        .filter_map(|p| std::fs::metadata(o.cache.join(format!("parks/{p}.fgpak"))).ok())
        .map(|m| m.len())
        .sum();
    println!(
        "[assets] locations: {} ({} KiB of park packs), default {default}",
        locations(default).join(", "),
        total / 1024
    );
    let districts: Vec<String> = game_core::menu::parse_index(&text)
        .locations
        .iter()
        .filter(|l| l.district.is_some())
        .map(|l| l.id.clone())
        .collect();
    if !districts.is_empty() {
        println!("[assets] districts (streamed): {}", districts.join(", "));
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
            kind: "thing",
            name: "thing".into(),
            files: vec!["a.fgpak".into(), "b.fgpak".into()],
            dir: None,
            args: vec!["--x".into()],
        }
    }

    #[test]
    fn a_district_installs_its_whole_folder() {
        let job = district_job("DownTown");
        assert_eq!(job.files, vec!["city/DownTown/index.fgpak".to_string()]);
        let root = tmp("district");
        let (cache, dest) = (root.join("cache"), root.join("dest"));
        let folder = cache.join("city/DownTown");
        std::fs::create_dir_all(&folder).unwrap();
        for f in ["index.fgpak", "c_50_50.fgpak", "c_-50_50.fgpak"] {
            std::fs::write(folder.join(f), b"cell").unwrap();
        }
        std::fs::write(stamp_path(&cache, &job), "v1").unwrap();
        assert!(up_to_date(&cache, &job, "v1"));
        assert!(install(&cache, &dest, &job, "v1").unwrap());
        assert!(dest.join("city/DownTown/c_-50_50.fgpak").is_file());
        assert!(dest.join("city/DownTown.stamp").is_file());
        assert!(!install(&cache, &dest, &job, "v1").unwrap());
        // A new conversion without a cell: the app folder loses it too.
        std::fs::remove_file(folder.join("c_50_50.fgpak")).unwrap();
        std::fs::write(stamp_path(&cache, &job), "v2").unwrap();
        assert!(install(&cache, &dest, &job, "v2").unwrap());
        assert!(!dest.join("city/DownTown/c_50_50.fgpak").exists());
        assert!(dest.join("city/DownTown/c_-50_50.fgpak").is_file());
        // A cell missing from the app folder is copied again, even with the same stamp.
        std::fs::remove_file(dest.join("city/DownTown/c_-50_50.fgpak")).unwrap();
        assert!(install(&cache, &dest, &job, "v2").unwrap());
        assert!(dest.join("city/DownTown/c_-50_50.fgpak").is_file());
        let _ = std::fs::remove_dir_all(&root);
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
        for f in &job.files {
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
    fn every_location_gets_a_job_and_an_index_line() {
        let all = jobs(DEFAULT_PARK);
        let parks: Vec<_> = all.iter().filter(|j| j.kind == "park").collect();
        assert_eq!(parks.len(), park::LOCATIONS.len());
        assert_eq!(parks[0].name, format!("parks/{DEFAULT_PARK}"));
        assert_eq!(parks[0].files, [format!("parks/{DEFAULT_PARK}.fgpak")]);
        // A park left out of the list is still converted when asked for, and comes first.
        let l = locations("SkateSchool");
        assert_eq!(l[0], "SkateSchool");
        assert_eq!(l.len(), park::LOCATIONS.len() + 1);
        assert_eq!(locations("MegaPark").len(), park::LOCATIONS.len());

        // Packs in a sub-folder install with their stamps next to them; the index lists them.
        let root = tmp("parks");
        let (cache, dest) = (root.join("cache"), root.join("dest"));
        let job = park_job("MegaPark");
        std::fs::create_dir_all(cache.join("parks")).unwrap();
        std::fs::write(cache.join("parks/MegaPark.fgpak"), vec![0u8; 1234]).unwrap();
        std::fs::write(stamp_path(&cache, &job), "s").unwrap();
        assert!(up_to_date(&cache, &job, "s"));
        assert!(install(&cache, &dest, &job, "s").unwrap());
        assert!(dest.join("parks/MegaPark.stamp").is_file());
        let index = game_core::menu::parse_index(&super::index(&cache, "MegaPark"));
        assert_eq!(index.default, Some(0));
        let m = &index.locations[0];
        assert_eq!(
            (m.id.as_str(), m.name.as_str(), m.bytes),
            ("MegaPark", "Mega Park", 1234)
        );
        assert_eq!(m.file, "parks/MegaPark.fgpak");
        assert_eq!(index.locations.len(), park::LOCATIONS.len());

        // A converted district: one location per start, after the parks.
        let city = cache.join("city/DownTown");
        std::fs::create_dir_all(&city).unwrap();
        let starts = vec![
            (game_core::math::v3(1.0, 0.0, 2.0), 0.0),
            (game_core::math::v3(300.0, 5.0, 2.0), 1.0),
        ];
        let i = game_core::city::Index {
            cell: 100.0,
            floor: -3.0,
            starts,
            cells: vec![],
        };
        crate::district::write_index(&city, &i).unwrap();
        std::fs::write(city.join("c_50_50.fgpak"), vec![0u8; 100]).unwrap();
        let index = game_core::menu::parse_index(&super::index(&cache, "MegaPark"));
        let d: Vec<_> = index
            .locations
            .iter()
            .filter_map(|l| Some((l.id.as_str(), l.name.as_str(), l.district?, l.file.as_str())))
            .collect();
        assert_eq!(
            d,
            [
                ("DownTown", "Downtown", 0, "city/DownTown/index.fgpak"),
                ("DownTown-2", "Downtown 2", 1, "city/DownTown/index.fgpak"),
            ]
        );
        let bytes = std::fs::metadata(city.join("index.fgpak")).unwrap().len() + 100;
        assert!(
            index
                .locations
                .iter()
                .all(|l| l.district.is_none() || l.bytes as u64 == bytes)
        );
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
