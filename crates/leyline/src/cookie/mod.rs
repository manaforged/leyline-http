#![forbid(unsafe_code)]
mod jar;
pub(crate) mod parse;
mod record;

pub use jar::{Jar, JarAutosave};
pub use record::{Cookie, SameSite};
