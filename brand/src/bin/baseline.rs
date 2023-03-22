/*
 * Copyright 2023 Oxide Computer Company
 */

use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;
use helios_build_utils::*;
use helios_omicron_brand::*;

/**
 * Try to unlink a file.  If it did not exist, treat that as a success; report
 * any other error.
 */
fn maybe_unlink(f: &Path) -> Result<()> {
    match std::fs::remove_file(f) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => bail!("could not remove {f:?}: {e:?}"),
    }
}

/**
 * Remove and recreate the SMF profile selection link with this name (e.g.,
 * "platform") and this target (e.g., "platform_none").  The full path of the
 * file will be resolved under root; e.g.,
 * "/some/root/etc/svc/profile/platform.xml".
 */
fn profile_link(root: &Path, name: &str, target: &str) -> Result<()> {
    let name = {
        let mut f = root.to_path_buf();
        f.push("etc");
        f.push("svc");
        f.push("profile");
        f.push(&format!("{name}.xml"));
        f
    };

    maybe_unlink(&name)?;
    std::os::unix::fs::symlink(format!("{target}.xml"), &name)?;

    Ok(())
}

fn main() -> Result<()> {
    let mut opts = getopts::Options::new();
    opts.optopt("R", "", "target image", "PATH");

    let mat = opts.parse(std::env::args().skip(1))?;

    if mat.free.len() != 1 {
        bail!("specify target directory for baseline");
    }

    let src = mat.opt_str("R").map(PathBuf::from);

    let dir = {
        let dir = PathBuf::from(&mat.free[0]);
        if dir.is_absolute() {
            dir
        } else {
            let mut cwd = std::env::current_dir()?;
            cwd.push(dir);
            cwd
        }
    };
    std::fs::create_dir_all(&dir)?;

    /*
     * Get the global zone version of the OS incorporation and entire packages.
     * In the production build this should be determined from the ramdisk root
     * for which we are generating the baseline archive.
     */
    let packages = pkg::pkg_list(src.as_ref())?;

    let incorp = packages
        .iter()
        .filter(|p| p.name() == "consolidation/osnet/osnet-incorporation")
        .collect::<Vec<_>>();
    if incorp.len() != 1 {
        bail!(
            "could not find single osnet-incorporation, got {:?}",
            incorp
        );
    }
    println!("incorp = {}", incorp[0]);

    let mut to_install = vec![incorp[0].clone()];

    match src.as_deref() {
        None => {
            /*
             * When operating against a development system, use the "entire"
             * metapackage for zone base contents.
             */
            let entire = packages
                .iter()
                .filter(|p| p.name() == "entire")
                .collect::<Vec<_>>();
            let entire = if entire.len() > 1 {
                bail!(
                    "could not find optional entire package, got {:?}",
                    entire
                );
            } else if entire.len() == 1 {
                entire[0].clone()
            } else {
                ips::Package::new_bare_version("entire", "latest")
            };

            println!("entire = {entire}");
            to_install.push(entire);

            /*
             * We need the SSH server in the switch zone for wicket, and chrony
             * in the NTP zone.  In the ramdisk environment these come along for
             * the ride in the image templates we are presently using, even
             * though they are otherwise optional in the "entire" metapackage.
             *
             * Until zone image construction more completely interacts with the
             * packaging system in order to request specific chunks of software
             * for inclusion in the image itself, we will include them in the
             * baseline.
             */
            packages
                .iter()
                .filter(|p| {
                    p.name() == "network/openssh-server"
                        || p.name() == "service/network/chrony"
                })
                .cloned()
                .for_each(|p| {
                    println!("install = {p}");
                    to_install.push(p)
                });
        }
        Some(src) => {
            /*
             * Otherwise, for ramdisk baseline construction, just use whatever
             * packages are now installed.
             */
            let contents = packages
                .iter()
                .filter(|p| {
                    p.name() != "entire"
                        && p.name() != "consolidation/osnet/osnet-incorporation"
                })
                .map(|p| {
                    Ok((p.clone(), pkg::pkg_contents(Some(src), Some(p))?))
                })
                .collect::<Result<Vec<_>>>()?;
            'pkgs: for (p, actions) in contents {
                for a in actions {
                    if let ips::ActionKind::Set(name, values) = a.kind() {
                        /*
                         * Packages may be marked for inclusion in a particular
                         * zone type; i.e., global or non-global.
                         */
                        let ngz = if name == "variant.opensolaris.zone" {
                            values.iter().any(|v| v == "nonglobal")
                        } else {
                            /*
                             * If there is no marking at all, assume the package
                             * is acceptable in a non-global zone.
                             */
                            true
                        };

                        if !ngz {
                            println!("skip global-only package = {p}");
                            continue 'pkgs;
                        }
                    }
                }

                println!("install = {p}");
                to_install.push(p)
            }
        }
    }

    /*
     * Create a temporary directory in which to assemble the image.
     *
     * Unfortunately the "tempfile" crate appears to make world-readable
     * temporary directories, which is less than ideal.  Create a user-only
     * subdirectory to avoid a race when trying to make the top-level directory
     * user-only.
     */
    let tmp = tempfile::TempDir::new()?;
    let tmp = {
        let mut tmp = tmp.path().to_path_buf();
        tmp.push("tmp");
        std::fs::DirBuilder::new().mode(0o700).create(&tmp)?;
        tmp
    };
    println!("tempdir @ {tmp:?}");

    /*
     * Currently pkg(1) seems to engage in some poorly considered behaviour.  It
     * will, with rmdir(2), attempt to remove its cache directory.  If there are
     * still files, obviously this will fail, but if empty, as it is immediately
     * after a "pkg image-create" with no publishers, it succeeds.  The program
     * then continues removing successive parent directories, even those we did
     * not ask it to create in the first place (!), right up until it cannot
     * remove /tmp because it is, mercifully, full of other people's files.
     *
     * This is a flat out, top-to-bottom flagrant error, and has the immediate
     * practical upshot of undoing our 0700 mode on the temporary directory by
     * removing and then recreating it with mode 0755.  The easiest spanner one
     * might furiously hurl into these particular works is to create an empty
     * file in the directory we so meticulously created so that rmdir(2) fails.
     */
    {
        let mut sadness = tmp.clone();
        sadness.push(".butwhy");
        std::fs::write(&sadness, b"")?;
    }

    /*
     * To complicate this further, we need _another_ layer of directory
     * underneath our protected temporary directory.  This is because the
     * directory at which we anchor the system image with "pkg image-create"
     * will subsequently be adjusted to be 0755, to match the expected and
     * packaged permissions of "/" on a UNIX system.
     */
    let root = {
        let mut root = tmp.clone();
        root.push("root");
        root
    };
    println!("image root @ {:?}", &root);
    let im = Some(&root);

    println!("creating image...");
    pkg::pkg_image_create(&root)?;

    /*
     * Copy publisher information from the running system.  In the production
     * build this should come from the ramdisk root for which we are generating
     * the baseline file:
     */
    println!("copying publishers...");
    let tmp;
    pkg::pkg_copy_publishers_from(
        im,
        if let Some(src) = src.as_ref() {
            src
        } else {
            tmp = PathBuf::from("/");
            &tmp
        },
    )?;

    /*
     * Tell IPS that we do not wish to include files under /usr, /sbin, or most
     * of /lib, in the resultant image:
     */
    println!("adding properties...");
    for pat in &["usr/", "sbin/", "lib/(?!svc/seed|svc/manifest)"] {
        pkg::pkg_add_property_value(im, "exclude-patterns", pat)?;
    }

    println!("installing packages...");
    pkg::pkg_exact_install(im, to_install.as_slice())?;

    println!("seeding SMF database...");
    let repodb = {
        let mut f = root.clone();
        f.push("etc");
        f.push("svc");
        f.push("repository.db");
        f
    };
    let mfdir = {
        let mut f = root.clone();
        f.push("lib");
        f.push("svc");
        f.push("manifest");
        f
    };
    maybe_unlink(&repodb)?;
    Command::new("/usr/sbin/svccfg")
        .env_clear()
        .env("SVCCFG_DTD", "/usr/share/lib/xml/dtd/service_bundle.dtd.1")
        .env("SVCCFG_REPOSITORY", &repodb)
        .env("SVCCFG_CHECKHASH", "1")
        .env("PKG_INSTALL_ROOT", &root)
        .arg("import")
        .arg("-p")
        .arg("-")
        .arg(&mfdir)
        .output()?
        .error_if_failed("svccfg")?;

    println!("configuring SMF profile...");
    {
        /*
         * Our brand-specific profile will disable by default some base OS
         * services that would otherwise run but which we do not need; e.g.,
         * "svc:/system/sac" or "svc:/network/inetd".
         *
         * If an image wants to further customise the default SMF posture, they
         * can provide a "/var/svc/profile/site.xml" file that will be processed
         * last; see smf_bootstrap(7).
         */
        let our_profile = {
            let mut f = root.clone();
            f.push("etc");
            f.push("svc");
            f.push("profile");
            f.push("platform_omicron1.xml");
            f
        };
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&our_profile)?;
        writeln!(f, "{}", include_str!("../../../config/profile.xml"))?;
        f.flush()?;
        f.sync_all()?;
    }
    profile_link(&root, "name_service", "ns_dns")?;
    profile_link(&root, "platform", "platform_omicron1")?;
    profile_link(&root, "inetd_services", "inetd_generic")?;
    profile_link(&root, "generic", "generic_limited_net")?;

    /*
     * Make the root account a no-password account; viz., one that will not
     * allow any password-based logins.
     */
    println!("modifying shadow(5)...");
    let shadowf = {
        let mut f = root.clone();
        f.push("etc");
        f.push("shadow");
        f
    };
    let mut shadow = unix::Shadow::load(&shadowf)?;
    shadow.password_set("root", "NP")?;
    unix::lchmod(&shadowf, 0o600)?;
    shadow.store(&shadowf)?;

    /*
     * The passwd(5) and group(5) databases have been populated by installed
     * packages.  Load them from the image so that we can translate user and
     * group names into user and group IDs that will make sense in the installed
     * environment.
     */
    println!("loading user and group database...");
    let passwdf = {
        let mut f = root.clone();
        f.push("etc");
        f.push("passwd");
        f
    };
    let passwd = unix::Passwd::load(&passwdf)?;
    let groupf = {
        let mut f = root.clone();
        f.push("etc");
        f.push("group");
        f
    };
    let group = unix::Group::load(&groupf)?;

    let out_tar = {
        let mut f = dir.clone();
        f.push("files.tar.gz");
        f
    };
    let out_gzonly = {
        let mut f = dir.clone();
        f.push("gzonly.txt");
        f
    };

    /*
     * Load the canonical list of packaged files from the image, including the
     * owner and group and mode bits.  We will use this metadata when creating
     * the tar file.  We will also use it to detect and handle files added to
     * the baseline beyond those tracked by the packaging system.
     */
    println!("assessing packaged files...");
    #[derive(Debug, PartialEq)]
    enum EntryType {
        Dir,
        File,
        Link(PathBuf),
        Hardlink(PathBuf),
    }
    struct Packaged {
        owner: String,
        group: String,
        mode: u32,
        etype: EntryType,
    }
    let mut gzonly_true = BTreeSet::new();
    let mut gzonly_false = BTreeSet::new();
    let mut packaged: BTreeMap<PathBuf, Packaged> = Default::default();
    for a in pkg::pkg_contents(im, None)? {
        use EntryType::*;
        let (etype, path) = match a.kind() {
            ips::ActionKind::File(af) => (File, af.path()),
            ips::ActionKind::Dir(af) => (Dir, af.path()),
            ips::ActionKind::Link(al) => {
                (Link(PathBuf::from(al.target())), al.path())
            }
            ips::ActionKind::Hardlink(al) => {
                (Hardlink(PathBuf::from(al.target())), al.path())
            }
            _ => continue,
        };
        let path = path.trim_start_matches('/').to_string();

        /*
         * Entries for the same path may end up several times in the full
         * contents if they are referenced by many packages (e.g., "/usr").  If
         * this path is mentioned in at least one package in an entry that does
         * not carry the global-zone-only variant, we will include it in the
         * baseline and will not attempt to exclude it from constructed zones.
         */
        match a.kind() {
            ips::ActionKind::File(_)
            | ips::ActionKind::Link(_)
            | ips::ActionKind::Hardlink(_) => {
                if a.gz_only() {
                    gzonly_true.insert(path.clone());
                } else {
                    gzonly_false.insert(path.clone());
                }
            }
            _ => {}
        }

        if a.gz_only() {
            continue;
        }

        let (owner, group, mode) = match a.kind() {
            ips::ActionKind::Dir(af) | ips::ActionKind::File(af) => {
                (af.owner().to_string(), af.group().to_string(), af.mode())
            }
            ips::ActionKind::Link(_) | ips::ActionKind::Hardlink(_) => {
                ("root".to_string(), "root".to_string(), 0)
            }
            _ => continue,
        };

        packaged.insert(
            PathBuf::from(path),
            Packaged {
                owner,
                group,
                mode,
                etype,
            },
        );
    }

    println!("creating archive...");

    maybe_unlink(&out_tar)?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&out_tar)?;
    let gzw = flate2::write::GzEncoder::new(f, flate2::Compression::best());
    let mut tar = tar::Builder::new(gzw);

    /*
     * Insert our metadata record as a regular file entry at the top of the
     * archive.  This file will not be extracted into the file system, but will
     * be read when inspecting the image to see if we understand the format.
     */
    metadata::MetadataBuilder::new(metadata::ArchiveType::Baseline)
        .build()?
        .append_to_tar(&mut tar)?;

    let mut found: BTreeMap<PathBuf, EntryType> = Default::default();
    let mut walk = walkdir::WalkDir::new(&root).min_depth(1).into_iter();
    while let Some(ent) = walk
        .next()
        .transpose()
        .with_context(|| anyhow!("walk of {:?}", &root))?
    {
        let p = tree::unprefix(&root, ent.path()).context("unprefix")?;
        assert!(!p.is_absolute());

        let md = ent.path().symlink_metadata()?;

        if md.file_type().is_symlink() {
            found.insert(
                p.to_path_buf(),
                EntryType::Link(ent.path().read_link()?),
            );
        } else if md.file_type().is_file() {
            found.insert(p.to_path_buf(), EntryType::File);
        } else if md.file_type().is_dir() {
            found.insert(p.to_path_buf(), EntryType::Dir);
        } else {
            bail!(
                "what type of file is {:?}, {:?}",
                ent.path(),
                md.file_type()
            );
        }
    }

    let mut tar_hardlinks: BTreeMap<PathBuf, tar::Header> = Default::default();
    let mut tar_symlinks: BTreeMap<PathBuf, tar::Header> = Default::default();
    let mut tar_files: BTreeMap<PathBuf, tar::Header> = Default::default();
    println!("missing from packaging:");
    for (p, et) in found.iter() {
        /*
         * Ignore IPS minutia that we do not wish to ship in the baseline:
         */
        if p.starts_with("var/cache/pkg")
            || p.starts_with("var/pkg")
            || p.starts_with("var/log/pkg")
            || p.starts_with("var/sadm/pkg")
            || p.starts_with("var/spool/pkg")
            || p.starts_with("var/sadm/install/contents")
        {
            continue;
        }

        let mut fullpath = root.clone();
        fullpath.push(p);

        /*
         * Files within the archive are stored under a "root/" prefix, in order
         * to distinguish them from metadata entries.  This prefix will be
         * stripped off during extraction.
         */
        let mut archivepath = PathBuf::from("root");
        archivepath.push(p);

        if let Some(pi) = packaged.get(p) {
            match (et, &pi.etype) {
                /*
                 * If a file is packaged as a hard link, it is difficult for us
                 * to be able to then see that just by looking at the file in
                 * the file system, so we allow this particular inconsistency.
                 */
                (EntryType::File, EntryType::Hardlink(_)) => {}
                (proto, pkg) => {
                    if proto != pkg {
                        bail!(
                            "path {:?} mismatched types: proto {:?}, pkg {:?}",
                            p,
                            et,
                            pi.etype
                        );
                    }
                }
            }

            match &pi.etype {
                EntryType::Dir => {
                    /*
                     * Put directories into the archive on the first pass.
                     */
                    let mut h = tar::Header::new_ustar();

                    /*
                     * Start with metadata from the file system, which includes
                     * mtime:
                     */
                    h.set_metadata(&fullpath.symlink_metadata()?);

                    /*
                     * Override with specifics from the packaging:
                     */
                    h.set_username(&pi.owner)?;
                    h.set_uid(passwd.lookup_by_name(&pi.owner)?);
                    h.set_groupname(&pi.group)?;
                    h.set_gid(group.lookup_by_name(&pi.group)?);
                    h.set_path(&archivepath)?;
                    h.set_mode(pi.mode);
                    h.set_cksum();

                    tar.append(&h, std::io::empty())?;
                }
                EntryType::File => {
                    let mut h = tar::Header::new_ustar();
                    h.set_metadata(&fullpath.symlink_metadata()?);
                    h.set_username(&pi.owner)?;
                    h.set_uid(passwd.lookup_by_name(&pi.owner)?);
                    h.set_groupname(&pi.group)?;
                    h.set_gid(group.lookup_by_name(&pi.group)?);
                    h.set_path(&archivepath)?;
                    h.set_mode(pi.mode);
                    h.set_cksum();
                    tar_files.insert(p.clone(), h);
                }
                EntryType::Link(target) => {
                    let mut h = tar::Header::new_ustar();
                    h.set_metadata(&fullpath.symlink_metadata()?);
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_username("root")?;
                    h.set_uid(passwd.lookup_by_name("root")?);
                    h.set_groupname("root")?;
                    h.set_gid(group.lookup_by_name("root")?);
                    h.set_link_name(target)?;
                    h.set_path(&archivepath)?;
                    h.set_cksum();
                    tar_symlinks.insert(p.clone(), h);
                }
                EntryType::Hardlink(target) => {
                    /*
                     * Fix up the hardlink path so that it is a root-anchored
                     * path like everything else.
                     */
                    let mut fullpath = root.clone();
                    fullpath.push(p.parent().ok_or(anyhow!("what?"))?);
                    fullpath.push(target);
                    let canon = fullpath.canonicalize()?;
                    let target = {
                        let mut target = PathBuf::from("root");
                        target.push(tree::unprefix(&root, &canon)?);
                        target
                    };
                    let mut h = tar::Header::new_ustar();
                    h.set_entry_type(tar::EntryType::Link);
                    h.set_link_name(&target)?;
                    h.set_path(&archivepath)?;
                    h.set_mode(0); /* XXX? */
                    h.set_size(0); /* XXX? */
                    h.set_uid(0); /* XXX? */
                    h.set_gid(0); /* XXX? */
                    h.set_cksum();
                    tar_hardlinks.insert(p.clone(), h);
                }
            }
        } else {
            /*
             * The package metadata does not describe this file.  We'll need to
             * fabricate permissions for it.
             */
            println!("    {p:?}");

            match et {
                EntryType::Dir => {
                    let mut h = tar::Header::new_ustar();

                    h.set_metadata(&fullpath.symlink_metadata()?);

                    h.set_username("root")?;
                    h.set_uid(passwd.lookup_by_name("root")?);
                    h.set_groupname("sys")?;
                    h.set_gid(group.lookup_by_name("sys")?);
                    h.set_path(&archivepath)?;
                    h.set_cksum();

                    tar.append(&h, std::io::empty())?;
                }
                EntryType::File => {
                    let mut h = tar::Header::new_ustar();
                    h.set_metadata(&fullpath.symlink_metadata()?);
                    h.set_username("root")?;
                    h.set_uid(passwd.lookup_by_name("root")?);
                    h.set_groupname("sys")?;
                    h.set_gid(group.lookup_by_name("sys")?);
                    h.set_path(&archivepath)?;
                    h.set_cksum();
                    tar_files.insert(p.clone(), h);
                }
                EntryType::Link(target) => {
                    let mut h = tar::Header::new_ustar();
                    h.set_metadata(&fullpath.symlink_metadata()?);
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_username("root")?;
                    h.set_uid(passwd.lookup_by_name("root")?);
                    h.set_groupname("root")?;
                    h.set_gid(group.lookup_by_name("root")?);
                    h.set_link_name(target)?;
                    h.set_path(&archivepath)?;
                    h.set_cksum();
                    tar_symlinks.insert(p.clone(), h);
                }
                x => bail!("unexpected file type found {:?}, {:?}", p, x),
            }
        }
    }
    println!();
    let mut header = false;
    let mut fail = false;
    for (p, _i) in packaged.iter() {
        if p.starts_with("usr")
            || p.starts_with("sbin")
            || (p.starts_with("lib")
                && !(p.starts_with("lib/svc/seed")
                    || p.starts_with("lib/svc/manifest")))
        {
            /*
             * Ignore these trees.  They will come from the ramdisk at zone
             * install time.  We asked IPS not to include them in the baseline
             * so they will not be present in the file system.
             */
            continue;
        }

        if !found.contains_key(p) {
            /*
             * This condition is unexpected; the packaging system expects
             * a file that we could not find in the proto area.
             */
            if !header {
                println!("missing from file system:");
                header = true;
            }
            println!("    {p:?}");
            fail = true;
        }
    }
    println!();

    if fail {
        bail!("cannot proceed due to issues with the proto area");
    }

    /*
     * Once all directories have been included in the archive, we can then
     * include files as we can be sure the directory in which they reside has
     * been properly created.
     */
    println!("finishing archive...");
    for (p, h) in tar_files.iter() {
        let mut fullpath = root.clone();
        fullpath.push(p);
        let f = std::fs::File::open(&fullpath)?;
        tar.append(h, f)?;
    }
    /*
     * After files comes hardlinks, as these links refer to existing files we
     * have already unpacked and need to exist at link(2) time.
     */
    for (_, h) in tar_hardlinks.iter() {
        tar.append(h, std::io::empty())?;
    }
    /*
     * Finally, include any symbolic links in the archive.
     */
    for (_, h) in tar_symlinks.iter() {
        tar.append(h, std::io::empty())?;
    }

    let gzw = tar.into_inner()?;
    let mut f = gzw.finish()?;
    f.flush()?;

    println!("creating gzonly manifest...");
    maybe_unlink(&out_gzonly)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&out_gzonly)?;
    for p in gzonly_true.iter().rev() {
        if p.starts_with("dev") {
            /*
             * Ignore /dev and /devices completely.  These are handled in a
             * zone-specific way.
             */
            continue;
        }

        if !gzonly_false.contains(p) {
            writeln!(f, "{p}")?;
        }
    }
    f.flush()?;
    f.sync_all()?;

    println!("ok");
    Ok(())
}
