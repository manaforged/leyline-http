use foreign_types::ForeignTypeRef;

use crate::ffi;
use crate::ssl::SslRef;

impl SslRef {
    /// Drop the handshake configuration after the handshake, and do not renegotiate.
    pub fn shed_handshake_config(&mut self) {
        unsafe {
            ffi::SSL_set_renegotiate_mode(self.as_ptr(), ffi::ssl_renegotiate_mode_t(0));
            ffi::SSL_set_shed_handshake_config(self.as_ptr(), 1);
        }
    }
}

#[cfg(test)]
#[path = "handshake_shed_tests.rs"]
mod handshake_shed_tests;
