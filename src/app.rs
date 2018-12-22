use std::path::Path;

use clap::App;

static ABOUT: &str = "
A command-line tool for organizing files using links.

The purpose of `hoard` is to make it easier to organize and access
static files such as videos and ebooks without needing to use an
heavy-weight GUI application. Instead, everything is managed using
filesystem native constructs. Files can be categorized into folders,
but can be associated with multiple folders through the use of links.

Use `init` to create a new hoard.";

pub fn app() -> App<'static, 'static> {
    clap_app!(hoard =>
        (@setting SubcommandRequiredElseHelp)
        (version: "0.1.0")
        (author: "Stephen Goeppele <s.goeppele.parrish@gmail.com>")
        (about: ABOUT)
        (@subcommand init =>
            (about: "Creates a new hoard")
            (@arg NAME: "the name of the hoard"))
        (@subcommand add =>
            (about: "Adds objects to the hoard")
            (@arg PATH: ... {path_exists} "the path of the object"))
        (@subcommand mv =>
            (about: "Renames objects in the hoard")
            (@arg NAME: ... "the unique name of the object"))
        (@subcommand rm =>
            (about: "Removes objects from the hoard")
            (@arg NAME: ... "the unique name of the object"))
        (@subcommand apply =>
            (about: "Syncs the repo to the index"))
        (@subcommand edit =>
            (about: "Opens an editor and syncs the repo to the index"))
        (@subcommand info =>
            (about: "Lists information about an object")
            (@arg OBJECT: "the path, name, or hash of the object"))
        (@subcommand query =>
            (about: "Lists all objects that match the query")))
}

fn path_exists(input: String) -> std::result::Result<(), String> {
    if Path::new(&input).exists() {
        Ok(())
    } else {
        Err(format!("pathspec '{}' did not match any files", input))
    }
}

