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
}

#[derive(Default)]
struct CopyQueueInner {
    cv: Condvar,
    locked: Mutex<CopyQueueLocked>,
}

#[derive(Default)]
struct CopyQueueLocked {
    fin: bool,
    q: Vec<CopyEntry>,
}

impl CopyQueue {
    /**
     * Create a thread pool and work queue for copying files.
     */
    pub fn new(threads: usize) -> Result<CopyQueue> {
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
        })
    }

    /**
     * Schedules a file copy operation in the thread pool and returns
     * immediately.
     */
    pub fn push_copy(&self, src: PathBuf, dst: PathBuf) {
        let mut locked = self.inner.locked.lock().unwrap();
        locked.q.push(CopyEntry { src, dst });
        self.inner.cv.notify_one();
    }

    /**
     * Waits for all enqueued file copies to complete and all of the threads in
     * the thread pool to exit.  Returns statistics about the copied files,
     * aggregated from all worker threads.
     */
    pub fn join(self) -> Result<CopyStats> {
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

struct CopyEntry {
    src: PathBuf,
    dst: PathBuf,
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
        let CopyEntry { src, dst } = {
            let mut locked = cqi.locked.lock().unwrap();
            loop {
                if let Some(ce) = locked.q.pop() {
                    break ce;
                } else {
                    if locked.fin {
                        return Ok(cs);
                    }

                    locked = cqi.cv.wait(locked).unwrap();
                    continue;
                }
            }
        };

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
         * Create the target file with the correct mode.  We made sure to remove
         * it earlier, so we should make sure we are creating it anew here.
         */
        let fdst = std::fs::OpenOptions::new()
            .mode(md.mode())
            .create_new(true)
            .write(true)
            .open(&dst)
            .map_err(mkerror)?;

        /*
         * The easiest way to copy a file, std::fs::copy(), appears to use a
         * regrettably microscopic buffer for reads and writes.  To make things
         * go quite a lot faster, create a buffered reader and writer with a
         * large buffer and use std::io::copy() instead, which will size read
         * and write calls based on that buffer size.
         */
        let cap = 1024 * 1024;
        let mut bsrc = std::io::BufReader::with_capacity(cap, fsrc);
        let mut bdst = std::io::BufWriter::with_capacity(cap, fdst);

        cs.bytes += std::io::copy(&mut bsrc, &mut bdst).map_err(mkerror)?;
    }
}
