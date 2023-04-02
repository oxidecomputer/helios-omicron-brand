/*
 * Copyright 2023 Oxide Computer Company
 */

use anyhow::{anyhow, bail, Result};
use std::fs::File;
use std::io::{Read, Seek};
use std::os::unix::prelude::*;
use std::path::{Path, PathBuf};

use helios_build_utils::{metadata, tree};

pub struct Unpack {
    gz: Option<flate2::read::GzDecoder<std::fs::File>>,
    archive: PathBuf,
    metadata: Option<metadata::Metadata>,
    opened: bool,
}

fn lstat<P: AsRef<Path>>(p: P) -> Result<Option<std::fs::Metadata>> {
    let p = p.as_ref();

    Ok(match p.symlink_metadata() {
        Ok(md) => Some(md),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => bail!("lstat({:?}): {:?}", p, e),
    })
}

impl Unpack {
    pub fn load<P: AsRef<Path>>(archive: P) -> Result<Unpack> {
        let archive = archive.as_ref().to_path_buf();

        let mut u = Unpack {
            gz: None,
            metadata: None,
            archive,
            opened: false,
        };
        u.load_metadata()
            .map_err(|e| anyhow!("loading archive {:?}: {e:?}", &u.archive))?;

        Ok(u)
    }

    pub fn metadata(&self) -> &metadata::Metadata {
        self.metadata.as_ref().unwrap()
    }

    fn open_tar(
        &mut self,
    ) -> Result<tar::Archive<&mut flate2::read::GzDecoder<std::fs::File>>> {
        let mut f = if let Some(gz) = self.gz.take() {
            gz.into_inner()
        } else {
            /*
             * We should only have to open the file once as we keep the file
             * descriptor around as long as this object exists.
             */
            assert!(!self.opened);
            self.opened = true;

            File::open(&self.archive)?
        };
        f.rewind()?;
        self.gz = Some(flate2::read::GzDecoder::new(f));
        Ok(tar::Archive::new(self.gz.as_mut().unwrap()))
    }

    fn load_metadata(&mut self) -> Result<()> {
        let mut tar = self.open_tar()?;

        /*
         * Locate the primary metadata file within the archive:
         */
        let mut ents = tar.entries()?;
        while let Some(mut ent) = ents.next().transpose()? {
            if !ent.header().entry_type().is_file() {
                /*
                 * Metadata must be in a regular file.
                 */
                continue;
            }

            let path = ent.path()?;
            if let Some(path) = path.to_str() {
                if path != "oxide.json" {
                    continue;
                }
            } else {
                continue;
            }

            let mut s = String::new();
            ent.read_to_string(&mut s)?;

            self.metadata = Some(metadata::parse(&s)?);

            return Ok(());
        }

        bail!("could not find metadata file, \"oxide.json\", in archive");
    }

    pub fn unpack<P: AsRef<Path>>(&mut self, outdir: P) -> Result<()> {
        let outdir = outdir.as_ref();
        let mut tar = self.open_tar()?;

        if !outdir.exists() {
            std::fs::create_dir(outdir)?;
        }

        let root_prefix = PathBuf::from("root");

        let mut ents = tar.entries()?;
        while let Some(mut ent) = ents.next().transpose()? {
            let h = ent.header();
            let p = ent.path()?;

            let mode = h.mode()?;
            let uid = h.uid()? as u32;
            let gid = h.gid()? as u32;

            if !p.starts_with(&root_prefix) {
                continue;
            }

            let target = tree::reprefix(&root_prefix, &p, outdir)?;
            let md = lstat(&target)?;

            match h.entry_type() {
                tar::EntryType::Regular
                | tar::EntryType::Symlink
                | tar::EntryType::Link => {
                    if let Some(md) = &md {
                        /*
                         * We are trying to create a regular file or a symlink,
                         * but the target path exists already.  Make sure it is
                         * already either a regular file or a symlink.
                         */
                        if !md.file_type().is_file()
                            && !md.file_type().is_symlink()
                        {
                            bail!(
                                "conflict: path {:?} is a {:?}, \
                                not a file or symlink",
                                target,
                                md.file_type(),
                            );
                        }

                        /*
                         * Unlink the existing file or symlink so that we can
                         * replace it with the contents from the archive.
                         */
                        std::fs::remove_file(&target)?;
                    }
                }
                _ => {}
            }

            let (chmod, chown) = match h.entry_type() {
                tar::EntryType::Directory => {
                    if let Some(md) = &md {
                        /*
                         * The path exists already.  Check to make sure it is a
                         * directory.
                         */
                        if !md.file_type().is_dir() {
                            bail!(
                                "conflict: path {:?} is a {:?}, not a dir",
                                target,
                                md.file_type(),
                            );
                        }

                        /*
                         * We need to update the metadata if it is not already
                         * correct:
                         */
                        (md.mode() != mode, md.uid() != uid || md.gid() != gid)
                    } else {
                        std::fs::create_dir(&target)?;
                        (true, true)
                    }
                }
                tar::EntryType::Regular => {
                    let mut f = std::fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&target)?;
                    std::io::copy(&mut ent, &mut f)?;
                    (true, true)
                }
                tar::EntryType::Symlink => {
                    let linktarget = ent.link_name()?.unwrap();

                    std::os::unix::fs::symlink(&linktarget, &target)?;

                    /*
                     * Symbolic links do not have permissions, and the default
                     * ownership of "root" is generally acceptable.
                     */
                    (false, false)
                }
                tar::EntryType::Link => {
                    let linktarget = tree::reprefix(
                        &root_prefix,
                        &ent.link_name()?.unwrap(),
                        outdir,
                    )?;

                    std::fs::hard_link(&linktarget, &target)?;

                    /*
                     * Permissions are per-inode, not per path, so we assume
                     * they were correctly set on the original file and leave
                     * them alone here.
                     */
                    (false, false)
                }
                x => bail!("unsupported entry type {:?}: {:?}", x, h),
            };

            if chmod {
                crate::unix::lchmod(&target, mode)?;
            }

            if chown {
                crate::unix::lchown(&target, uid, gid)?;
            }
        }

        Ok(())
    }
}
