#[macro_use]
extern crate clap;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_derive;

extern crate hex;
extern crate multi_map;
extern crate open;
extern crate pathdiff;
extern crate regex;
extern crate serde;
extern crate serde_json;
extern crate sha2;
extern crate walkdir;

use std::env;
use std::fs;
use std::process;

use clap::ArgMatches;

use hoard::Repository;

mod app;
mod error;
mod hoard;
mod state;

pub type Result<T> = ::std::result::Result<T, failure::Error>;

fn main() {
    if let Err(e) = try_main() {
        eprintln!("{}", e);
        process::exit(1);
    }
}

fn try_main() -> Result<()> {
    match app::app().get_matches().subcommand() {
        ("init", Some(matches)) => init(matches),
        ("add", Some(matches)) => add(matches),
        ("edit", Some(matches)) => edit(matches),
        ("apply", Some(matches)) => apply(matches),
        (command, _) => bail!("'{}' not implemented", command),
    }
}

fn init(matches: &ArgMatches) -> Result<()> {
    let path = matches.value_of("NAME").unwrap_or(".");
    Repository::init(&path)?;
    println!(
        "Initialized new hoard repository in {}",
        fs::canonicalize(&path)?.display()
    );
    Ok(())
}

fn add(matches: &ArgMatches) -> Result<()> {
    let current_dir = env::current_dir()?;
    let mut repo = Repository::load(current_dir)?;
    if let Some(paths) = matches.values_of("PATH") {
        repo.add(paths.collect())?;
    }
    Ok(())
}

fn edit(_matches: &ArgMatches) -> Result<()> {
    Ok(())
}

fn apply(_: &ArgMatches) -> Result<()> {
    let current_dir = env::current_dir()?;
    let repo = Repository::load(current_dir)?;
    repo.apply()
}
