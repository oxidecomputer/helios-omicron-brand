/*
 * Copyright 2024 Oxide Computer Company
 */

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::copyq::{CopyQueue, CopyStats};
use crate::defaults::DefaultsFile;

pub fn unprefix(prefix: &Path, path: &Path) -> Result<PathBuf> {
    if prefix.is_absolute() != path.is_absolute() {
        bail!("prefix and path must not be a mix of absolute and relative");
    }

    let cprefix = prefix.components().collect::<Vec<_>>();
    let cpath = path.components().collect::<Vec<_>>();

    if let Some(tail) = cpath.strip_prefix(cprefix.as_slice()) {
        Ok(tail.iter().collect())
    } else {
        bail!("{:?} does not start with prefix {:?}", path, prefix);
    }
}

pub fn reprefix(prefix: &Path, path: &Path, target: &Path) -> Result<PathBuf> {
    if !target.is_absolute() {
        bail!("target must be absolute");
    }
    let mut newpath = target.to_path_buf();
    newpath.push(unprefix(prefix, path)?);
    Ok(newpath)
}

/**
 * Replicate "src" (e.g., "/usr") as a tree of symlinks rooted at "target"
 * (e.g., "/zone/root/usr") where each link will point at the lofs file system
 * pointed at "prefix" (e.g., "/system/usr").
 */
pub fn replicate<S: AsRef<Path>, T: AsRef<Path>>(
    src: S,
    target: T,
    prefix: &str,
) -> Result<CopyStats> {
    let src = src.as_ref();
    let target = target.as_ref();

    if !src.is_absolute() || !src.exists() {
        bail!("src {:?} must exist and be absolute", src);
    }
    if !target.is_absolute() || !target.exists() {
        bail!("target {:?} must exist and be absolute", target);
    }
    if !prefix.starts_with('/') {
        bail!("prefix must be absolute");
    }

    let df = DefaultsFile::from_path("/etc/default/helios-omicron1")?;

    let mut cq = CopyQueue::new(
        df.get_usize("COPY_THREADS").unwrap_or(8),
        df.get_usize("COPY_BATCH").unwrap_or(128),
    )?;

    let walk = walkdir::WalkDir::new(src).same_file_system(true);
    let mut walk = walk.into_iter();

    while let Some(ent) = walk.next().transpose()? {
        let md = ent.metadata()?;

        if md.file_type().is_symlink() {
            /*
             * We recreate relative symbolic links in the target tree with the
             * same contents as in the source tree.  Both relative and absolute
             * links will continue to point to the correct place when examined
             * in the context of the zone, provided all of the replicated trees
             * are laid out in the usual locations.
             */

            let target = reprefix(src, ent.path(), target)?;
            cq.push_relative_link(ent.path().into(), target);
        } else if md.file_type().is_dir() {
            /*
             * Just create directories with the same ownership and permissions
             * as the original.
             */
            let target = reprefix(src, ent.path(), target)?;
            if target.exists() && target.is_dir() {
                continue;
            }
            std::fs::create_dir(&target)
                .with_context(|| anyhow!("creating {:?}", &target))?;
        } else if md.file_type().is_file() {
            if ent.file_name().to_string_lossy().contains(".so")
                || ent.path().to_string_lossy().contains("usr/bin")
                || ent.path().to_string_lossy().contains("usr/libexec")
                || ent.path().to_string_lossy().contains("usr/share/man")
                || ent.path().to_string_lossy().contains("usr/share/locale")
                || ent.path().to_string_lossy().contains("sbin")
            {
                /*
                 * XXX Try the symlink thing with library or program files.
                 */

                /*
                 * Create an absolute symbolic link to the analogous file in the
                 * prefix tree.
                 */
                let target = reprefix(src, ent.path(), target)?;

                /*
                 * XXX This is rubbish:
                 */
                let mut linktarget = String::new();
                for _ in 0..ent.depth() {
                    if !linktarget.is_empty() {
                        linktarget.push('/');
                    }
                    linktarget.push_str("..");
                }
                linktarget.push_str(prefix);
                linktarget.push('/');
                linktarget
                    .push_str(unprefix(src, ent.path())?.to_str().unwrap());

                cq.push_absolute_link(linktarget, target);
            } else {
                /*
                 * XXX Copy the analogous file to the prefix tree:
                 */
                let target = reprefix(src, ent.path(), target)?;

                /*
                 * Push the copy task onto the work queue and move on to the
                 * next file.
                 */
                cq.push_copy(ent.path().into(), target);
            }
        } else {
            bail!("special file? {:?}", ent.path());
        }
    }

    Ok(cq.join()?)
}
