/*
 * Copyright 2023 Oxide Computer Company
 */

use anyhow::{bail, Result};
use std::collections::BTreeSet;
use std::convert::{TryFrom, TryInto};
use std::fmt::Display;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    name: String,
    publisher: Option<String>,
    version: Option<String>,
    date: Option<String>,
}

impl Display for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pkg:/")?;
        if let Some(p) = &self.publisher {
            write!(f, "/{p}/")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(v) = &self.version {
            write!(f, "@{v}")?;
            if let Some(d) = &self.date {
                write!(f, ":{d}")?;
            }
        }
        Ok(())
    }
}

impl Package {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn publisher(&self) -> Option<&str> {
        self.publisher.as_deref()
    }

    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }

    pub fn new_bare(name: &str) -> Package {
        Package {
            name: name.to_string(),
            publisher: None,
            version: None,
            date: None,
        }
    }

    pub fn new_bare_version(name: &str, version: &str) -> Package {
        Package {
            name: name.to_string(),
            publisher: None,
            version: Some(version.to_string()),
            date: None,
        }
    }

    /**
     * Parse an FMRI from a depend action.  Apparently partial package names are
     * assumed to be anchored at the publisher root.
     */
    pub fn parse_fmri(fmri: &str) -> Result<Package> {
        let (publisher, input) = if let Some(i) = fmri.strip_prefix("pkg:/") {
            /*
             * Check to see if we expect a publisher.
             */
            if let Some(i) = i.strip_prefix('/') {
                let mut p = String::new();
                let mut r = String::new();
                let mut inpub = true;
                for c in i.chars() {
                    if inpub {
                        if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                            p.push(c);
                        } else if c == '/' {
                            if p.is_empty() {
                                bail!("expected publisher in \"{}\"", fmri);
                            }
                            inpub = false;
                        } else {
                            bail!("expected \"{}\" in \"{}\"", c, fmri);
                        }
                    } else {
                        r.push(c);
                    }
                }
                (Some(p), r)
            } else {
                (None, i.into())
            }
        } else if let Some(i) = fmri.strip_prefix('/') {
            if i.starts_with('/') {
                bail!("unexpected publisher without pkg: in \"{}\"", fmri);
            }
            (None, i.into())
        } else {
            (None, fmri.into())
        };

        let t = input.split('@').collect::<Vec<_>>();
        let (name, version, date) = match t.as_slice() {
            [n] => (n.to_string(), None, None),
            [n, x] => {
                let t = x.split(':').collect::<Vec<_>>();
                match t.as_slice() {
                    [v] => (n.to_string(), Some(v.to_string()), None),
                    [v, d] => (
                        n.to_string(),
                        Some(v.to_string()),
                        Some(d.to_string()),
                    ),
                    _ => bail!("too much : in \"{}\"", fmri),
                }
            }
            _ => bail!("too much @ in \"{}\"", fmri),
        };

        Ok(Package {
            name,
            publisher,
            version,
            date,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependType {
    Incorporate,
    Require,
    RequireAny,
    Group,
    GroupAny,
    Optional,
    Conditional,
}

impl TryFrom<String> for DependType {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<DependType> {
        s.as_str().try_into()
    }
}

impl TryFrom<&str> for DependType {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<DependType> {
        Ok(match s {
            "incorporate" => DependType::Incorporate,
            "require" => DependType::Require,
            "require-any" => DependType::RequireAny,
            "group" => DependType::Group,
            "group-any" => DependType::GroupAny,
            "optional" => DependType::Optional,
            "conditional" => DependType::Conditional,
            n => bail!("unknown depend type {:?}", n),
        })
    }
}

impl Display for DependType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            DependType::Incorporate => "incorporate",
            DependType::Require => "require",
            DependType::RequireAny => "require-any",
            DependType::Group => "group",
            DependType::GroupAny => "group-any",
            DependType::Optional => "optional",
            DependType::Conditional => "conditional",
        };
        write!(f, "{s}")
    }
}

pub struct Action {
    kind: ActionKind,
    #[allow(unused)]
    vals: Vals,
    variant_zone: Option<String>,
    #[allow(unused)]
    variant_imagetype: Option<String>,
}

impl Action {
    pub fn kind(&self) -> &ActionKind {
        &self.kind
    }

    pub fn gz_only(&self) -> bool {
        self.variant_zone
            .as_deref()
            .map(|zone| zone == "global")
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub struct ActionDepend {
    fmri: Vec<Package>,
    type_: DependType,
    predicate: Vec<String>,
}

impl ActionDepend {
    pub fn fmris(&self) -> &[Package] {
        self.fmri.as_slice()
    }

    pub fn predicate(&self) -> &[String] {
        self.predicate.as_slice()
    }

    pub fn type_(&self) -> DependType {
        self.type_
    }
}

#[derive(Debug, Clone)]
pub struct ActionLink {
    path: String,
    target: String,
}

impl ActionLink {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

#[derive(Debug, Clone)]
pub struct ActionFile {
    path: String,
    owner: String,
    group: String,
    mode: u32,
    fileid: Option<String>,
}

impl ActionFile {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn fileid(&self) -> Option<&str> {
        self.fileid.as_deref()
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn group(&self) -> &str {
        &self.group
    }
}

#[derive(Debug, Clone)]
pub enum ActionKind {
    Set(String, Vec<String>),
    Depend(ActionDepend),
    Unknown(String, Vec<String>),
    File(ActionFile),
    Dir(ActionFile),
    Link(ActionLink),
    Hardlink(ActionLink),
}

#[derive(Debug)]
enum ParseState {
    Rest,
    Type,
    Key,
    Value,
    ValueQuoted,
    ValueQuotedSpace,
    ValueUnquoted,
}

#[derive(Debug, Clone)]
pub struct Vals {
    vals: Vec<(String, String)>,
    extra: BTreeSet<String>,
}

impl Vals {
    fn new() -> Vals {
        Vals {
            vals: Vec::new(),
            extra: BTreeSet::new(),
        }
    }

    fn insert(&mut self, key: &str, value: &str) {
        /*
         * XXX Ignore "facet.*" properties for now...
         */
        if key.starts_with("facet.") {
            return;
        }

        self.vals.push((key.to_string(), value.to_string()));
        self.extra.insert(key.to_string());
    }

    fn maybe_single(&mut self, name: &str) -> Result<Option<String>> {
        let mut out: Option<String> = None;

        for (k, v) in self.vals.iter() {
            if k == name {
                if out.is_some() {
                    bail!(
                        "more than one value for {}, wanted a single value",
                        name
                    );
                }
                out = Some(v.to_string());
            }
        }

        self.extra.remove(name);
        Ok(out)
    }

    fn single(&mut self, name: &str) -> Result<String> {
        let out = self.maybe_single(name)?;

        if let Some(out) = out {
            Ok(out)
        } else {
            bail!("no values for {} found", name);
        }
    }

    fn maybe_list(&mut self, name: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();

        for (k, v) in self.vals.iter() {
            if k == name {
                out.push(v.to_string());
            }
        }

        self.extra.remove(name);
        out
    }

    fn list(&mut self, name: &str) -> Result<Vec<String>> {
        let out = self.maybe_list(name);
        if out.is_empty() {
            bail!("wanted at least one value for {}, found none", name);
        }
        Ok(out)
    }

    fn check_for_extra(&self) -> Result<()> {
        if !self.extra.is_empty() {
            bail!(
                "some properties present but not consumed: {:?}, {:?}",
                self.extra,
                self.vals
            );
        }

        Ok(())
    }
}

pub fn parse_manifest(input: &str) -> Result<Vec<Action>> {
    let mut out = Vec::new();

    for l in input.lines() {
        let mut s = ParseState::Rest;
        let mut a = String::new();
        let mut k = String::new();
        let mut v = String::new();
        let mut vals = Vals::new();
        let mut free: Vec<String> = Vec::new();
        let mut quote = '"';

        for c in l.chars() {
            match s {
                ParseState::Rest => {
                    if c.is_ascii_alphabetic() {
                        a.clear();
                        k.clear();
                        v.clear();

                        a.push(c);
                        s = ParseState::Type;
                    } else {
                        bail!("invalid line ({:?}): {}", s, l);
                    }
                }
                ParseState::Type => {
                    if c.is_ascii_alphabetic() {
                        a.push(c);
                    } else if c == ' ' {
                        s = ParseState::Key;
                    } else {
                        bail!("invalid line ({:?}): {}", s, l);
                    }
                }
                ParseState::Key => {
                    if c.is_ascii_alphanumeric()
                        || c == '.'
                        || c == '-'
                        || c == '_'
                        || c == '/'
                        || c == '@'
                        || c == '+'
                    {
                        k.push(c);
                    } else if c == ' ' {
                        free.push(k.clone());
                        k.clear();
                    } else if c == '=' {
                        s = ParseState::Value;
                    } else {
                        bail!("invalid line ({:?}, {}): {}", s, k, l);
                    }
                }
                ParseState::Value => {
                    /*
                     * This state represents the start of a new value, which
                     * will either be quoted or unquoted.
                     */
                    v.clear();
                    if c == '"' || c == '\'' {
                        /*
                         * Record the type of quote used at the start of the
                         * string so that we can match it with the same type
                         * of quote at the end.
                         */
                        quote = c;
                        s = ParseState::ValueQuoted;
                    } else {
                        s = ParseState::ValueUnquoted;
                        v.push(c);
                    }
                }
                ParseState::ValueQuoted => {
                    if c == '\\' {
                        /*
                         * XXX handle escaped quotes...
                         */
                        bail!("invalid line (backslash...): {}", l);
                    } else if c == quote {
                        s = ParseState::ValueQuotedSpace;
                    } else {
                        v.push(c);
                    }
                }
                ParseState::ValueQuotedSpace => {
                    /*
                     * We expect at least one space after a quoted string before
                     * the next key.
                     */
                    if c == ' ' {
                        vals.insert(&k, &v);
                        s = ParseState::Key;
                        k.clear();
                    } else {
                        bail!("invalid after quote ({:?}, {}): {}", s, k, l);
                    }
                }
                ParseState::ValueUnquoted => {
                    if c == '"' || c == '\'' {
                        bail!("invalid line (errant quote...): {}", l);
                    } else if c == ' ' {
                        vals.insert(&k, &v);
                        s = ParseState::Key;
                        k.clear();
                    } else {
                        v.push(c);
                    }
                }
            }
        }

        match s {
            ParseState::ValueQuotedSpace | ParseState::ValueUnquoted => {
                vals.insert(&k, &v);
            }
            ParseState::Type => {}
            _ => bail!("invalid line (terminal state {:?}: {}", s, l),
        }

        let variant_zone = vals.maybe_single("variant.opensolaris.zone")?;
        let variant_imagetype =
            vals.maybe_single("variant.opensolaris.imagetype")?;
        let kind = match a.as_str() {
            "set" => {
                let name = vals.single("name")?;
                let values = vals.list("value")?;
                vals.check_for_extra()?;

                ActionKind::Set(name, values)
            }
            "depend" => {
                let fmri = vals
                    .list("fmri")?
                    .iter()
                    .map(|fmri| Package::parse_fmri(fmri.as_str()))
                    .collect::<Result<Vec<_>>>()?;
                let type_ = vals.single("type")?.try_into()?;
                let predicate = vals.maybe_list("predicate");
                /*
                 * XXX Ignore...
                 */
                vals.maybe_single("pkg.linted")?;

                vals.check_for_extra()?;

                ActionKind::Depend(ActionDepend {
                    fmri,
                    type_,
                    predicate,
                })
            }
            "dir" => {
                let path = vals.single("path")?;
                if !free.is_empty() {
                    bail!("should not have a fileid? {:?}", free);
                }
                let owner = vals.single("owner")?;
                let group = vals.single("group")?;
                let mode = u32::from_str_radix(&vals.single("mode")?, 8)?;
                ActionKind::Dir(ActionFile {
                    path,
                    owner,
                    group,
                    mode,
                    fileid: None,
                })
            }
            "file" => {
                let path = vals.single("path")?;
                if free.len() > 1 {
                    bail!("more than one fileid? {:?}", free);
                }
                let fileid = free.pop();
                let owner = vals.single("owner")?;
                let group = vals.single("group")?;
                let mode = u32::from_str_radix(&vals.single("mode")?, 8)?;
                ActionKind::File(ActionFile {
                    path,
                    owner,
                    group,
                    mode,
                    fileid,
                })
            }
            "link" => {
                let path = vals.single("path")?;
                let target = vals.single("target")?;
                if !free.is_empty() {
                    bail!("spare arguments? {:?}", free);
                }
                ActionKind::Link(ActionLink { path, target })
            }
            "hardlink" => {
                let path = vals.single("path")?;
                let target = vals.single("target")?;
                if !free.is_empty() {
                    bail!("spare arguments? {:?}", free);
                }
                ActionKind::Hardlink(ActionLink { path, target })
            }
            _ => ActionKind::Unknown(a.to_string(), free),
        };

        out.push(Action {
            kind,
            vals,
            variant_zone,
            variant_imagetype,
        });
    }

    Ok(out)
}
