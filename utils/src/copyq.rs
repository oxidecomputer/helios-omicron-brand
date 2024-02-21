/*
 * Copyright 2024 Oxide Computer Company
 */

use anyhow::{anyhow, Result};

use std::{
    os::unix::prelude::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

pub struct CopyQueue {
    inner: Arc<CopyQueueInner>,
    threads: Vec<thread::JoinHandle<std::result::Result<CopyStats, String>>>,
    pending: Vec<CopyEntry>,
    batch: usize,
}

#[derive(Default)]
struct CopyQueueInner {
    cv: Condvar,
    locked: Mutex<CopyQueueLocked>,
}

#[derive(Default)]
struct CopyQueueLocked {
    fin: bool,
    q: Vec<Vec<CopyEntry>>,
}

impl CopyQueue {
    /**
     * Create a thread pool and work queue for copying files.
     */
    pub fn new(threads: usize, batch: usize) -> Result<CopyQueue> {
        let cqi = Arc::new(CopyQueueInner::default());

        let threads = (0..threads)
            .map(|_| {
                let cqi = Arc::clone(&cqi);
                Ok(thread::Builder::new()
                    .name("copy".into())
                    .spawn(|| copy_thread(cqi))?)
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(CopyQueue {
            inner: cqi,
            threads,
            pending: vec![],
            batch,
        })
    }

    pub fn dispatch(&mut self) {
        let pending = std::mem::replace(&mut self.pending, Vec::new());
        let mut locked = self.inner.locked.lock().unwrap();
        locked.q.push(pending);
        self.inner.cv.notify_one();
    }

    /**
     * Schedules a file copy operation in the thread pool and returns
     * immediately.
     */
    pub fn push_copy(&mut self, src: PathBuf, dst: PathBuf) {
        self.pending.push(CopyEntry::Copy { src, dst });

        if self.pending.len() == self.batch {
            self.dispatch();
        }
    }

    pub fn push_relative_link(&mut self, src: PathBuf, dst: PathBuf) {
        self.pending.push(CopyEntry::RelativeLink { src, dst });

        if self.pending.len() == self.batch {
            self.dispatch();
        }
    }

    pub fn push_absolute_link(&mut self, src: String, dst: PathBuf) {
        self.pending.push(CopyEntry::AbsoluteLink { src, dst });

        if self.pending.len() == self.batch {
            self.dispatch();
        }
    }

    /**
     * Waits for all enqueued file copies to complete and all of the threads in
     * the thread pool to exit.  Returns statistics about the copied files,
     * aggregated from all worker threads.
     */
    pub fn join(mut self) -> Result<CopyStats> {
        self.dispatch();

        /*
         * Inform the worker threads that there are no more files to copy:
         */
        self.inner.locked.lock().unwrap().fin = true;
        self.inner.cv.notify_all();

        let mut tcs = CopyStats::default();

        for t in self.threads {
            let cs = t
                .join()
                .unwrap()
                .map_err(|e| anyhow!("copy thread: {e:?}"))?;

            tcs.files += cs.files;
            tcs.bytes += cs.bytes;
        }

        Ok(tcs)
    }
}

enum CopyEntry {
    Copy { src: PathBuf, dst: PathBuf },
    RelativeLink { src: PathBuf, dst: PathBuf },
    AbsoluteLink { src: String, dst: PathBuf },
}

#[derive(Default, Debug)]
pub struct CopyStats {
    pub files: u64,
    pub bytes: u64,
}

fn copy_thread(
    cqi: Arc<CopyQueueInner>,
) -> std::result::Result<CopyStats, String> {
    let mut cs = CopyStats { files: 0, bytes: 0 };

    loop {
        let cv = {
            let mut locked = cqi.locked.lock().unwrap();
            loop {
                if let Some(cv) = locked.q.pop() {
                    break cv;
                } else {
                    if locked.fin {
                        return Ok(cs);
                    }

                    locked = cqi.cv.wait(locked).unwrap();
                    continue;
                }
            }
        };

        for work in cv {
            match work {
                CopyEntry::Copy { src, dst } => {
                    let mkerror = |e| format!("copy {src:?} -> {dst:?}: {e}");

                    cs.files += 1;
                    std::fs::remove_file(&dst).ok();

                    let fsrc = std::fs::OpenOptions::new()
                        .read(true)
                        .open(&src)
                        .map_err(mkerror)?;
                    let md = fsrc.metadata().map_err(mkerror)?;
                    assert!(md.is_file());

                    /*
                     * Create the target file with the correct mode.  We made
                     * sure to remove it earlier, so we should make sure we are
                     * creating it anew here.
                     */
                    let fdst = std::fs::OpenOptions::new()
                        .mode(md.mode())
                        .create_new(true)
                        .write(true)
                        .open(&dst)
                        .map_err(mkerror)?;

                    /*
                     * The easiest way to copy a file, std::fs::copy(), appears
                     * to use a regrettably microscopic buffer for reads and
                     * writes.  To make things go quite a lot faster, create a
                     * buffered reader and writer with a large buffer and use
                     * std::io::copy() instead, which will size read and write
                     * calls based on that buffer size.
                     */
                    let cap = 1024 * 1024;
                    let mut bsrc = std::io::BufReader::with_capacity(cap, fsrc);
                    let mut bdst = std::io::BufWriter::with_capacity(cap, fdst);

                    cs.bytes +=
                        std::io::copy(&mut bsrc, &mut bdst).map_err(mkerror)?;
                }

                CopyEntry::RelativeLink { src, dst } => {
                    let mke = |e| format!("rel link {src:?} -> {dst:?}: {e}");

                    let linktarget = std::fs::read_link(&src).map_err(mke)?;

                    /*
                     * XXX remove first...
                     */
                    std::os::unix::fs::symlink(&linktarget, &dst)
                        .map_err(mke)?;
                }

                CopyEntry::AbsoluteLink { src, dst } => {
                    let mke = |e| format!("abs link {src:?} -> {dst:?}: {e}");

                    std::os::unix::fs::symlink(&src, &dst).map_err(mke)?;
                }
            }
        }
    }
}
