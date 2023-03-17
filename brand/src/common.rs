use anyhow::{anyhow, bail, Result};

pub fn argv(i: usize, m: &str) -> Result<String> {
    std::env::args()
        .nth(i + 1)
        .ok_or_else(|| anyhow!("need argument for {}", m))
}

pub trait OutputExt {
    fn error_if_failed(&self, msg: &str) -> Result<()>;
}

impl OutputExt for std::process::Output {
    fn error_if_failed(&self, msg: &str) -> Result<()> {
        if !self.status.success() {
            bail!(
                "{} failure: {:?}",
                msg,
                String::from_utf8_lossy(&self.stderr).trim()
            );
        } else {
            Ok(())
        }
    }
}
