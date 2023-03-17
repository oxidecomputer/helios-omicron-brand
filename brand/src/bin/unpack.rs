/*
 * Copyright 2023 Oxide Computer Company
 */

use anyhow::Result;

use common::*;
use helios_omicron_brand::*;

fn main() -> Result<()> {
    let mut image = unpack::Unpack::load(argv(0, "image file path")?)?;
    let outdir = argv(1, "output directory")?;

    println!("metadata: {:?}", image.metadata());

    image.unpack(&outdir)?;

    Ok(())
}
