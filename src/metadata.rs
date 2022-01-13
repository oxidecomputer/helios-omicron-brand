use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveType {
    Baseline,
    Layer,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub v: String,
    pub t: ArchiveType,
}

pub fn parse(s: &str) -> Result<Metadata> {
    Ok(serde_json::from_str(s)?)
}

impl Metadata {
    pub fn append_to_tar<T: std::io::Write>(
        &self,
        a: &mut tar::Builder<T>,
    ) -> Result<()> {
        let b = serde_json::to_vec(self)?;

        let mut h = tar::Header::new_ustar();
        h.set_username("root")?;
        h.set_uid(0);
        h.set_groupname("root")?;
        h.set_gid(0);
        h.set_path("oxide.json")?;
        h.set_mode(0o444);
        h.set_size(b.len().try_into().unwrap());
        h.set_cksum();

        a.append(&h, b.as_slice())?;
        Ok(())
    }

    pub fn is_layer(&self) -> bool {
        matches!(&self.t, ArchiveType::Layer)
    }

    pub fn is_baseline(&self) -> bool {
        matches!(&self.t, ArchiveType::Baseline)
    }
}
