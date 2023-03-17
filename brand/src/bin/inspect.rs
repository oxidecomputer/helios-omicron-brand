/*
 * Copyright 2023 Oxide Computer Company
 */

use anyhow::Result;

use common::*;
use helios_omicron_brand::*;

fn main() -> Result<()> {
    let image = unpack::Unpack::load(argv(0, "image file path")?)?;

    println!("metadata: {:?}", image.metadata());

    Ok(())
}
