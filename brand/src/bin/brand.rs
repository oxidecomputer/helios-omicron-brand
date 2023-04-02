/*
 * Copyright 2023 Oxide Computer Company
 */

use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{anyhow, bail, Result};

use helios_build_utils::*;
use helios_omicron_brand::*;

#[allow(unused)]
mod ids {
    pub const ROOT: u32 = 0;
    pub const BIN: u32 = 2;
    pub const SYS: u32 = 3;
}
use ids::*;

#[derive(Debug)]
enum PrestateCurrent {
    Installed,
    Ready,
    Running,
    ShuttingDown,
    Down,
    Mounted,
}

impl FromStr for PrestateCurrent {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "2" => PrestateCurrent::Installed,
            "3" => PrestateCurrent::Ready,
            "4" => PrestateCurrent::Running,
            "5" => PrestateCurrent::ShuttingDown,
            "6" => PrestateCurrent::Down,
            "7" => PrestateCurrent::Mounted,
            other => bail!("unknown current state {:?}", other),
        })
    }
}

#[derive(Debug)]
enum PrestateCommand {
    Ready,
    Boot,
    Halt,
}

impl FromStr for PrestateCommand {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "0" => PrestateCommand::Ready,
            "1" => PrestateCommand::Boot,
            "4" => PrestateCommand::Halt,
            other => bail!("unknown transition command {:?}", other),
        })
    }
}

#[allow(dead_code)]
struct Stuff {
    zone: String,
    zonepath: PathBuf,
    debug: bool,
}

impl Stuff {
    fn zoneroot(&self) -> PathBuf {
        self.otherdir("root")
    }

    fn zonerootpath(&self, names: &[&str]) -> PathBuf {
        let mut dir = self.zoneroot();
        names.iter().for_each(|name| dir.push(name));
        dir
    }

    fn otherdir(&self, name: &str) -> PathBuf {
        let mut dir = self.zonepath.clone();
        dir.push(name);
        dir
    }

    fn baseline(&self, name: &str) -> Result<PathBuf> {
        const DIRS: &[&str] = &[
            "/var/run/brand/omicron1/baseline",
            "/usr/lib/brand/omicron1/baseline",
        ];

        for &dir in DIRS {
            let mut p = PathBuf::from(dir);
            p.push(name);
            if p.exists() {
                return Ok(p);
            }
        }

        bail!("could not locate {:?} in any baseline directory", name);
    }
}

fn debug_from_env() -> bool {
    match std::env::var("DEBUG_OMICRON_BRAND") {
        Ok(val) if val == "1" || val == "true" || val == "yes" => true,
        _ => false,
    }
}

fn mkstuff(m: &getopts::Matches) -> Result<Stuff> {
    Ok(Stuff {
        debug: debug_from_env(),
        zone: m.opt_str("z").ok_or(anyhow!("-z required"))?,
        zonepath: PathBuf::from(m.opt_str("R").ok_or(anyhow!("-R required"))?),
    })
}

fn cmd_prestate(
    s: Stuff,
    args: &mut dyn Iterator<Item = &String>,
) -> Result<()> {
    let ps: PrestateCurrent = args.next().ok_or(anyhow!("arg1"))?.parse()?;
    let pc: PrestateCommand = args.next().ok_or(anyhow!("arg2"))?.parse()?;
    let mp = args.next().map(PathBuf::from);

    if s.debug {
        println!("INFO: prestate {ps:?} {pc:?} {mp:?}");
    }

    Ok(())
}

fn cmd_poststate(
    s: Stuff,
    args: &mut dyn Iterator<Item = &String>,
) -> Result<()> {
    let ps: PrestateCurrent = args.next().ok_or(anyhow!("arg1"))?.parse()?;
    let pc: PrestateCommand = args.next().ok_or(anyhow!("arg2"))?.parse()?;
    let mp = args.next().map(PathBuf::from);

    if s.debug {
        println!("INFO: poststate {ps:?} {pc:?} {mp:?}");
    }

    Ok(())
}

fn cmd_query(_s: Stuff, args: &mut dyn Iterator<Item = &String>) -> Result<()> {
    let _q = args.next().ok_or(anyhow!("wanted a query argument"))?;

    /*
     * XXX Apparently we should not fail, but rather emit nothing on stdout if
     * we do not recognise the query.
     *
     * The one query we know about so far is "datasets" which we should
     * probably implement.
     */

    Ok(())
}

fn cmd_verify_cfg(args: &mut dyn Iterator<Item = &String>) -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.parsing_style(getopts::ParsingStyle::StopAtFirstFree);
    let mat = opts.parse(args)?;

    if mat.free.len() != 1 {
        bail!("expected a temporary XML file path");
    }
    let xmlpath = PathBuf::from(&mat.free[0]);

    if debug_from_env() {
        println!("XML file @ {xmlpath:?}");

        let mut f = std::fs::File::open(&xmlpath)?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        for l in s.lines() {
            println!("  | {l}");
        }
    }

    /*
     * XXX For now, we are not going to do anything here.  We should probably
     * look at the XML file, though, and ensure that, e.g., "ip-type" is set to
     * "exclusive" amongst other things.
     */

    Ok(())
}

fn cmd_install(
    s: Stuff,
    args: &mut dyn Iterator<Item = &String>,
) -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.parsing_style(getopts::ParsingStyle::StopAtFirstFree);
    let mat = opts.parse(args)?;

    println!(
        "INFO: omicron: installing zone {} @ {:?}...",
        s.zone, s.zonepath,
    );

    /*
     * We need to create the "root" directory within the zonepath as part of
     * installation.
     */
    let root = s.zoneroot();
    std::fs::DirBuilder::new().mode(0o755).create(&root)?;
    unix::lchown(&root, ROOT, ROOT)?;

    for repl in ["usr", "lib", "sbin"] {
        let tree = format!("/{repl}");
        println!("INFO: omicron: replicating {tree} tree...");
        let dir = s.zonerootpath(&[repl]);
        std::fs::DirBuilder::new().mode(0o755).create(&dir)?;
        unix::lchown(&dir, ROOT, SYS)?;
        tree::replicate(&tree, &dir, &format!("/system/{repl}"))?;
    }

    {
        /*
         * For now, just remove all manifests from the global zone and use the
         * ones from the baseline package.  This will match the preseed database
         * exactly, and no stragglers will slip through.
         */
        println!("INFO: omicron: pruning SMF manifests...");
        let manifest = s.zonerootpath(&["lib", "svc", "manifest"]);
        std::fs::remove_dir_all(&manifest)?;
    }

    /*
     * Remove any files that the baseline says are global-zone only.
     */
    println!("INFO: omicron: pruning global-only files...");
    for l in std::fs::read_to_string(s.baseline("gzonly.txt")?)?.lines() {
        let mut rm = root.clone();
        let rel = PathBuf::from(l);
        if rel.is_absolute() {
            bail!("absolute path in baseline remove list: {rel:?}");
        }
        rm.push(rel);

        if let Err(e) = std::fs::remove_file(&rm) {
            if e.kind() != std::io::ErrorKind::NotFound {
                bail!("removing {rm:?}: {e:?}");
            }
        } else if s.debug {
            println!("removed GZ-only file: {rm:?}");
        }
    }

    /*
     * Unpack the baseline archive into the zone root, which will establish the
     * contents of /etc, /var, and /root, and /lib/svc/seed/nonglobal.db:
     */
    println!("INFO: omicron: unpacking baseline archive...");
    let mut baseline = unpack::Unpack::load(s.baseline("files.tar.gz")?)?;
    if !baseline.metadata().is_baseline() {
        bail!("archive is not a baseline archive");
    }
    baseline.unpack(&root)?;

    /*
     * Unpack any additional archives that were passed on the command line:
     */
    for extra in mat.free {
        println!("INFO: omicron: unpacking image {extra:?}...");
        let mut extra = unpack::Unpack::load(&extra)?;
        if !extra.metadata().is_layer() {
            bail!("image is not a layer");
        }
        extra.unpack(&root)?;
    }

    println!("INFO: omicron: install complete, probably!");

    Ok(())
}

fn cmd_uninstall(
    s: Stuff,
    args: &mut dyn Iterator<Item = &String>,
) -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.parsing_style(getopts::ParsingStyle::StopAtFirstFree);
    opts.optflag("F", "", "force");
    let mat = opts.parse(args)?;

    if !mat.free.is_empty() {
        bail!("unexpected arguments {:?}", mat.free);
    }

    println!("INFO: omicron: uninstalling zone {}...", s.zone);

    /*
     * XXX It would seem it is our responsibility to destroy the dataset or
     * zonepath directory.
     */
    let root = s.zoneroot();
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }

    Ok(())
}

fn main() -> Result<()> {
    /*
     * Parse global options first.  The first free argument will be a command,
     * and there may then be further option arguments after that.
     */
    let mut opts = getopts::Options::new();
    opts.parsing_style(getopts::ParsingStyle::StopAtFirstFree);
    opts.optopt("z", "", "zone name", "NAME");
    opts.optopt("R", "", "zone path", "DIR");

    /*
     * XXX Take this camera.  I want to document everything!
     */
    {
        let allargs = std::env::args().collect::<Vec<_>>();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/var/log/omicron-brand.log")?;
        writeln!(log, "{} [{}] {:?}", now, std::process::id(), allargs)?;
        log.flush()?;
    }

    let mat = opts.parse(std::env::args().skip(1))?;

    let mut args = mat.free.iter();
    match args.next().map(|x| x.as_str()) {
        Some("verify_cfg") => cmd_verify_cfg(&mut args),
        Some("verify_adm") => Ok(()),
        Some("query") => cmd_query(mkstuff(&mat)?, &mut args),
        Some("install") => cmd_install(mkstuff(&mat)?, &mut args),
        Some("uninstall") => cmd_uninstall(mkstuff(&mat)?, &mut args),
        Some("prestatechange") => cmd_prestate(mkstuff(&mat)?, &mut args),
        Some("poststatechange") => cmd_poststate(mkstuff(&mat)?, &mut args),
        other => {
            bail!("unrecognised command {:?}", other);
        }
    }
}
