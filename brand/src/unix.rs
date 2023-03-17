use anyhow::{bail, Result};
use std::ffi::CString;
use std::io::Write;
use std::path::Path;

pub fn lchmod<P: AsRef<Path>>(path: P, mode: u32) -> Result<()> {
    let path = path.as_ref();

    /*
     * This is not race free, but seems to be the best we can do?!
     */
    let md = std::fs::symlink_metadata(path)?;
    if md.file_type().is_symlink() {
        bail!("{:?} is a symbolic link", path);
    }

    let cname = CString::new(path.to_str().unwrap().to_string())?;

    /*
     * Regrettably, one apparently cannot use AT_SYMLINK_NOFOLLOW with
     * fchmodat(2), or we could avoid the race in the check above.
     */
    if unsafe { libc::fchmodat(libc::AT_FDCWD, cname.as_ptr(), mode, 0) } != 0 {
        let e = std::io::Error::last_os_error();
        bail!("lchmod({:?}, {}): {:?}", path, mode, e);
    }

    Ok(())
}

pub fn lchown<P: AsRef<Path>>(path: P, owner: u32, group: u32) -> Result<()> {
    let path = path.as_ref();
    let cname = CString::new(path.to_str().unwrap().to_string())?;

    if unsafe { libc::lchown(cname.as_ptr(), owner, group) } != 0 {
        let e = std::io::Error::last_os_error();
        bail!("lchown({:?}, {}, {}): {:?}", path, owner, group, e);
    }

    Ok(())
}

#[derive(Debug, PartialEq)]
pub struct Row {
    fields: Vec<String>,
}

#[derive(Debug, PartialEq)]
struct Database {
    name: String,
    nfields: usize,
    entries: Vec<Row>,
}

impl Database {
    fn load<P: AsRef<Path>>(
        name: &str,
        path: P,
        nfields: usize,
    ) -> Result<Database> {
        let name = name.to_string();
        let path = path.as_ref();
        let data = std::fs::read_to_string(path)?;

        let entries = data
            .lines()
            .enumerate()
            .map(|(i, l)| {
                if let Some(pfx) = l.chars().next() {
                    if pfx == '-' || pfx == '+' {
                        bail!(
                            "invalid {} line {}: compat not supported: {:?}",
                            name,
                            i,
                            l,
                        );
                    }
                }
                let fields =
                    l.split(':').map(str::to_string).collect::<Vec<_>>();
                if fields.len() != nfields {
                    bail!("invalid {} line {}: {:?}", name, i, fields);
                }
                Ok(Row { fields })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Database {
            name,
            nfields,
            entries,
        })
    }

    pub fn store<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path.as_ref())?;

        let mut data = self
            .entries
            .iter()
            .map(|e| e.fields.join(":"))
            .collect::<Vec<_>>()
            .join("\n");
        data.push('\n');

        f.write_all(data.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        Ok(())
    }
}

pub struct Group {
    database: Database,
}

impl Group {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Passwd> {
        let database = Database::load("group", path, 4)?;

        Ok(Passwd { database })
    }

    pub fn store<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.database.store(path)
    }

    pub fn lookup_by_name(&self, n: &str) -> Result<u64> {
        let matches = self
            .database
            .entries
            .iter()
            .filter(|r| r.fields[0] == n)
            .collect::<Vec<_>>();
        match matches.len() {
            1 => Ok(matches[0].fields[2].parse()?),
            0 => bail!("could not find user {:?}", n),
            c => bail!("found {} matches for user {:?}", c, n),
        }
    }
}

pub struct Passwd {
    database: Database,
}

impl Passwd {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Passwd> {
        let database = Database::load("passwd", path, 7)?;

        Ok(Passwd { database })
    }

    pub fn store<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.database.store(path)
    }

    pub fn lookup_by_name(&self, n: &str) -> Result<u64> {
        let matches = self
            .database
            .entries
            .iter()
            .filter(|r| r.fields[0] == n)
            .collect::<Vec<_>>();
        match matches.len() {
            1 => Ok(matches[0].fields[2].parse()?),
            0 => bail!("could not find user {:?}", n),
            c => bail!("found {} matches for user {:?}", c, n),
        }
    }
}

pub struct Shadow {
    database: Database,
}

impl Shadow {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Shadow> {
        let database = Database::load("shadow", path, 9)?;

        Ok(Shadow { database })
    }

    pub fn password_set(&mut self, user: &str, password: &str) -> Result<()> {
        let mc = self
            .database
            .entries
            .iter()
            .filter(|r| r.fields[0] == user)
            .count();
        if mc != 1 {
            bail!("found {} matches for user {} in shadow file", mc, user);
        }

        self.database.entries.iter_mut().for_each(|r| {
            if r.fields[0] == user {
                r.fields[1] = password.to_string();
            }
        });
        Ok(())
    }

    pub fn store<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.database.store(path)
    }
}
