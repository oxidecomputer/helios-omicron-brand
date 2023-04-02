/*
 * Copyright 2023 Oxide Computer Company
 */

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};

use helios_build_utils::ips;

pub const ROOT_IMAGE: Option<&Path> = None;

const PKG: &str = "/usr/bin/pkg";

fn pkg<P: AsRef<Path>>(image: Option<P>, subcmd: &str) -> Command {
    let mut cmd = Command::new(PKG);
    cmd.env_clear();
    if let Some(image) = image {
        cmd.arg("-R");
        cmd.arg(image.as_ref());
    }
    cmd.arg(subcmd);
    cmd
}

pub fn pkg_exact_install<'a, I, P: AsRef<Path>>(
    image: Option<P>,
    packages: I,
) -> Result<()>
where
    I: IntoIterator<Item = &'a ips::Package>,
{
    let mut cmd = pkg(image, "exact-install");
    cmd.arg("--no-refresh");
    cmd.arg("--no-index");
    for p in packages.into_iter() {
        cmd.arg(p.to_string());
    }

    let res = cmd.output()?;

    if res.status.success() {
        Ok(())
    } else {
        bail!(
            "pkg error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

pub fn pkg_add_property_value<P: AsRef<Path>>(
    image: Option<P>,
    name: &str,
    value: &str,
) -> Result<()> {
    let res = pkg(image, "add-property-value")
        .arg(name)
        .arg(value)
        .output()?;

    if res.status.success() {
        Ok(())
    } else {
        bail!(
            "pkg error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

pub fn pkg_copy_publishers_from<P1, P2>(image: Option<P1>, w: P2) -> Result<()>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let res = pkg(image, "copy-publishers-from")
        .arg(w.as_ref())
        .output()?;

    if res.status.success() {
        Ok(())
    } else {
        bail!(
            "pkg error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

pub fn pkg_image_create<P: AsRef<Path>>(image: P) -> Result<()> {
    let res = pkg(None::<&str>, "image-create")
        .arg("--full")
        .arg("--zone")
        .arg(image.as_ref())
        .output()?;

    if res.status.success() {
        Ok(())
    } else {
        bail!(
            "pkg image-create error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

pub fn pkg_refresh<P: AsRef<Path>>(image: Option<P>) -> Result<()> {
    let res = pkg(image, "refresh").output()?;

    if res.status.success() {
        Ok(())
    } else {
        bail!(
            "pkg refresh error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

pub fn pkg_list<P: AsRef<Path>>(image: Option<P>) -> Result<Vec<ips::Package>> {
    let res = pkg(image, "contents")
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

pub fn pkg_contents<P: AsRef<Path>>(
    image: Option<P>,
    package: Option<&ips::Package>,
) -> Result<Vec<ips::Action>> {
    let mut cmd = pkg(image, "contents");
    cmd.arg("-m");
    if let Some(p) = package {
        cmd.arg(p.to_string());
    }

    let res = cmd.output()?;

    if res.status.success() {
        Ok(ips::parse_manifest(&String::from_utf8(res.stdout)?)?)
    } else {
        bail!(
            "pkg contents error: {:?}",
            String::from_utf8_lossy(&res.stderr).trim()
        );
    }
}

#[derive(Debug)]
pub enum ImageFileDetails {
    File {
        owner: String,
        group: String,
        mode: u32,
    },
    Hardlink {
        target: String,
    },
}

#[derive(Debug)]
pub struct ImageFile {
    pub name: PathBuf,
    pub package: ips::Package,
    pub details: ImageFileDetails,
}

pub fn pkg_all_files<P: AsRef<Path>>(
    image: Option<P>,
) -> Result<Vec<ImageFile>> {
    let res = pkg(image, "contents")
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
                bail!("weird line {:?}", t);
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
