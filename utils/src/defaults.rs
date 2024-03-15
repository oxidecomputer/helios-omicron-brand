/*
 * Copyright 2024 Oxide Computer Company
 */

use std::{collections::HashMap, mem, path::Path, str::FromStr};

use anyhow::{bail, Result};

/**
 * An extremely minimal parser for a subset of defaults files, as potentially
 * read by defopen() in "lib/libc/port/gen/deflt.c".
 */
#[derive(Debug)]
pub struct DefaultsFile {
    values: HashMap<String, String>,
}

impl DefaultsFile {
    pub fn get_usize<S: AsRef<str>>(&self, name: S) -> Option<usize> {
        self.values.get(name.as_ref()).and_then(|v| v.parse().ok())
    }

    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<DefaultsFile> {
        let path = path.as_ref();

        match std::fs::read_to_string(path) {
            Ok(s) => Ok(DefaultsFile::from_str(s.as_str())?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                /*
                 * If the file is merely missing, pretend it was empty instead.
                 */
                Ok(DefaultsFile {
                    values: Default::default(),
                })
            }
            Err(e) => bail!("could not load defaults from {path:?}: {e}"),
        }
    }
}

impl FromStr for DefaultsFile {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        enum State {
            Rest,
            Comment,
            Key,
            Value,
        }

        let mut l = 1;
        let mut s = State::Rest;
        let mut k = String::new();
        let mut v = String::new();
        let mut values = HashMap::new();

        for c in input.chars() {
            match s {
                State::Rest => {
                    if c == '#' {
                        s = State::Comment;
                    } else if c == '\n' {
                        l += 1;
                    } else if c.is_ascii_alphabetic() {
                        k.clear();
                        k.push(c);
                        s = State::Key;
                    } else {
                        bail!("unexpected character {c:?}, line {l}");
                    }
                }
                State::Comment => {
                    if c == '\n' {
                        l += 1;
                        s = State::Rest;
                    } else if c.is_ascii_control() && c != '\t' {
                        bail!("unexpected character {c:?}, line {l}");
                    }
                }
                State::Key => {
                    if c == '#' {
                        bail!("unexpected comment in key name, line {l}");
                    } else if c.is_ascii_alphanumeric() || c == '_' {
                        k.push(c);
                    } else if c == '=' {
                        v.clear();
                        s = State::Value;
                    } else {
                        bail!("unexpected character {c:?}, line {l}");
                    }
                }
                State::Value => {
                    if c == '#' || c == '\n' {
                        values.insert(
                            mem::take(&mut k),
                            mem::take(&mut v).trim().to_string(),
                        );
                        s = if c == '#' {
                            State::Comment
                        } else {
                            State::Rest
                        };
                    } else if c.is_ascii_graphic() || c == ' ' {
                        v.push(c);
                    }
                }
            }
        }

        Ok(DefaultsFile { values })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_brand_file() {
        let input = concat!(
            "#\n",
            "# Copyright 2024 Oxide Computer Company\n",
            "#\n",
            "\n",
            "#\n",
            "# How many writer threads should we use when copying\n",
            "# from the root file system in a new zone root?\n",
            "#\n",
            "COPY_THREADS=16\n",
            "\n",
            "#\n",
            "# What batch size should we use for the copy queue?\n",
            "#\n",
            "COPY_BATCH=32\n",
            "\n",
            "#\n",
            "# This is not well-formed.\n",
            "#\n",
            "COPY_WHO_I_AM=Mr. Stephens?! Head of Catering?!\n",
            "\n",
        );

        let df = DefaultsFile::from_str(&input).expect("parsed output");
        println!("df = {df:#?}");

        assert_eq!(df.get_usize("COPY_THREADS"), Some(16));
        assert_eq!(df.get_usize("COPY_BATCH"), Some(32));
        assert_eq!(df.get_usize("COPY_WHO_I_AM"), None);
    }
}
