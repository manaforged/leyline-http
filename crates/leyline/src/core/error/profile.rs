use std::fmt;

use super::Error;

#[derive(Debug)]
pub(crate) struct ProfileChanged;

impl fmt::Display for ProfileChanged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the session profile differs from the pinned profile")
    }
}

impl std::error::Error for ProfileChanged {}

impl Error {
    pub fn is_profile_changed(&self) -> bool {
        self.source_as::<ProfileChanged>().is_some()
    }
}
