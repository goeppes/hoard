use std::fmt;
use std::path::Path;
use std::result;

use failure::{self, Context};

pub trait ResultExt<T, E>: failure::ResultExt<T, E>
where
    E: fmt::Display,
{
    fn with_path<P: AsRef<Path>>(self, path: P) -> result::Result<T, Context<String>>
    where
        Self: Sized,
    {
        self.with_context(|e| format!("{}: {}", path.as_ref().display(), e))
    }
}

impl<T, E: fmt::Display> ResultExt<T, E> for result::Result<T, E> where
    result::Result<T, E>: failure::ResultExt<T, E>
{}
