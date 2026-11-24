use leyline_bssl::ssl::{
    AsyncPrivateKeyMethod, AsyncSelectCertError, BoxGetSessionFuture, BoxSelectCertFuture,
    ClientHello, SslContextBuilder, SslRef,
};

pub trait SslContextBuilderExt: private::Sealed {
    fn set_async_select_certificate_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ClientHello<'_>) -> Result<BoxSelectCertFuture, AsyncSelectCertError>
            + Send
            + Sync
            + 'static;

    fn set_async_private_key_method(&mut self, method: impl AsyncPrivateKeyMethod);

    /// Installs an asynchronous TLS session lookup callback on this context builder.
    ///
    /// # Safety
    ///
    /// Caller must uphold the safety contract of `SslContextBuilder::set_async_get_session_callback`.
    unsafe fn set_async_get_session_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut SslRef, &[u8]) -> Option<BoxGetSessionFuture> + Send + Sync + 'static;
}

impl SslContextBuilderExt for SslContextBuilder {
    fn set_async_select_certificate_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut ClientHello<'_>) -> Result<BoxSelectCertFuture, AsyncSelectCertError>
            + Send
            + Sync
            + 'static,
    {
        self.set_async_select_certificate_callback(callback);
    }

    fn set_async_private_key_method(&mut self, method: impl AsyncPrivateKeyMethod) {
        self.set_async_private_key_method(method);
    }

    unsafe fn set_async_get_session_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut SslRef, &[u8]) -> Option<BoxGetSessionFuture> + Send + Sync + 'static,
    {
        // SAFETY: caller upholds the `SslContextBuilder::set_async_get_session_callback` contract.
        unsafe {
            self.set_async_get_session_callback(callback);
        }
    }
}

mod private {
    pub trait Sealed {}
}

impl private::Sealed for SslContextBuilder {}
