/*
 * Copyright 2023 Oxide Computer Company
 */

/*
 * XXX This program is not finished.
 */

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use helios_build_utils::*;
use helios_omicron_brand::*;

const PKG: &str = "/usr/bin/pkg";

use anyhow::{bail, Result};

fn pkg_list<P: AsRef<Path>>(image: P) -> Result<Vec<ips::Package>> {
    let res = Command::new(PKG)
        .env_clear()
        .arg("-R")
        .arg(image.as_ref())
        .arg("contents")
        .arg("-H")
        .arg("-o")
        .arg("pkg.fmri")
        .arg("-t")
        .arg("set")
        .arg("-a")
        .arg("name=pkg.fmri")
        .output()?;

    if res.status.success() {
        let mut list: Vec<ips::Package> = String::from_utf8(res.stdout)?
            .lines()
            .map(ips::Package::parse_fmri)
            .collect::<Result<_>>()?;
        list.sort();
        Ok(list)
    } else {
        bail!(
            "pkg list error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

fn pkg_contents<P: AsRef<Path>>(
    image: P,
    package: &ips::Package,
) -> Result<Vec<ips::Action>> {
    let res = Command::new(PKG)
        .env_clear()
        .arg("-R")
        .arg(image.as_ref())
        .arg("contents")
        .arg("-m")
        .arg(package.to_string())
        .output()?;

    if res.status.success() {
        Ok(ips::parse_manifest(&String::from_utf8(res.stdout)?)?)
    } else {
        bail!(
            "pkg contents error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

#[allow(dead_code)]
#[derive(Debug)]
enum ImageFileDetails {
    File {
        owner: String,
        group: String,
        mode: u32,
    },
    Hardlink {
        target: String,
    },
}

#[allow(dead_code)]
#[derive(Debug)]
struct ImageFile {
    name: PathBuf,
    package: ips::Package,
    details: ImageFileDetails,
}

fn pkg_all_files<P: AsRef<Path>>(image: P) -> Result<Vec<ImageFile>> {
    let res = Command::new(PKG)
        .env_clear()
        .arg("-R")
        .arg(image.as_ref())
        .arg("contents")
        .arg("-H")
        .arg("-o")
        .arg("action.name,action.key,owner,group,mode,target,pkg.fmri")
        .arg("-t")
        .arg("file,link,dir,hardlink")
        .output()?;

    if res.status.success() {
        let stdout = String::from_utf8(res.stdout)?;
        let lines = stdout.lines().map(|l| l.split('\t').collect::<Vec<_>>());

        let mut out = Vec::new();
        for t in lines {
            if t.len() != 7 {
                bail!("weird line {t:?}");
            }

            let details = match t[0] {
                "file" => ImageFileDetails::File {
                    owner: t[2].to_string(),
                    group: t[3].to_string(),
                    mode: u32::from_str_radix(t[4], 8)?,
                },
                "hardlink" => ImageFileDetails::Hardlink {
                    target: t[5].to_string(),
                },
                _ => {
                    continue;
                }
            };

            out.push(ImageFile {
                name: PathBuf::from(t[1]),
                package: ips::Package::parse_fmri(t[6])?,
                details,
            });
        }
        Ok(out)
    } else {
        bail!(
            "pkg contents error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

fn main() -> Result<()> {
    let zi = PathBuf::from(argv(0, "zone image path")?);

    println!("zone image @ {zi:?}");

    /*
     * Get a list of all packages installed in the zone image:
     */
    let list = pkg_list(&zi)?;
    println!("list = {list:#?}");

    /*
     * Find the osnet-incorporation, which we will use to exclude ramdisk
     * packages:
     */
    let incorp = list
        .iter()
        .filter(|p| p.name() == "consolidation/osnet/osnet-incorporation")
        .collect::<Vec<_>>();
    if incorp.len() != 1 {
        bail!("could not find illumos incorporation; got {:?}", incorp);
    }

    /*
     * Make a list of packages that we believe should come from the ramdisk.
     * Start with anything that is included as a dependency of
     * "consolidation/osnet/osnet-incorporation", which is shipped from illumos:
     */
    let gatemf = pkg_contents(&zi, incorp[0])?;
    let mut names = gatemf
        .iter()
        .filter_map(|a| match a.kind() {
            ips::ActionKind::Depend(ad) => match ad.type_() {
                ips::DependType::Incorporate => Some(
                    ad.fmris()
                        .iter()
                        .map(|p| p.name().to_string())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .collect::<HashSet<_>>();

    /*
     * XXX Add some extra packages we know about:
     * XXX We should include everything that we know to be in the ramdisk, but
     * the ramdisk does not yet exist, so this is still guesswork.
     */
    names.insert("system/management/snmp/net-snmp".to_string());
    names.insert("release/name".to_string());
    names.insert("runtime/perl".to_string());
    names.insert("runtime/perl/module/sun-solaris".to_string());
    names.insert("library/libxml2".to_string());
    names.insert("library/zlib".to_string());
    names.insert("library/security/trousers".to_string());
    names.insert("shell/bash".to_string());
    names.insert("compress/bzip2".to_string());
    names.insert("compress/xz".to_string());
    //println!("names = {:#?}", names);

    /*
     * List all files in the image:
     */
    let files = pkg_all_files(&zi)?;
    println!("{} in total file list", files.len());

    let mut packaged = HashSet::new();
    for f in files.iter() {
        if f.name.is_absolute() {
            bail!("what? {:?}", f);
        }
        packaged.insert(f.name.to_path_buf());
    }

    /*
     * Make a list of all files in the image so that we can find the unpackaged
     * ones.
     */
    let mut wd = walkdir::WalkDir::new(&zi)
        .same_file_system(true)
        .into_iter();
    //let mut wd = wd.into_iter();
    let mut spares = Vec::new();
    while let Some(ent) = wd.next().transpose()? {
        /*
         * XXX look at symlinks also.
         */
        if ent.file_type().is_symlink() || !ent.file_type().is_file() {
            continue;
        }

        let rp = tree::unprefix(&zi, ent.path())?;

        /*
         * Skip packaged files.
         */
        if packaged.contains(&rp) {
            continue;
        }

        /*
         * XXX Ignore some files we just don't care about:
         */
        if rp.starts_with("var/pkg")
            || rp.starts_with("var/sadm")
            || rp == PathBuf::from("etc/.pwd.lock")
        {
            continue;
        }

        //println!("    {:?}", rp);
        spares.push(rp);
    }

    if !spares.is_empty() {
        bail!("unpackaged files found in the image: {:?}", spares);
    }

    /*
     * Make a plan for the tar file.
     */
    for f in files.iter() {
        if names.contains(f.package.name()) {
            continue;
        }
        //if !f.name.starts_with("usr") {
        println!("  i {:?}", f.name);
        //}
    }

    Ok(())
}
