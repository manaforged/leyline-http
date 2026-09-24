use crate::error::ErrorStack;
use crate::ffi;
use crate::stack::StackRef;
use crate::x509::verify::{X509VerifyFlags, X509VerifyParamRef};
use crate::x509::{X509Object, X509Ref};
use crate::{cvt, cvt_p};
use foreign_types::{ForeignType, ForeignTypeRef};
use openssl_macros::corresponds;
use std::mem::ManuallyDrop;

foreign_type_and_impl_send_sync! {
    type CType = ffi::X509_STORE;
    fn drop = ffi::X509_STORE_free;

    pub struct X509StoreBuilder;
}

impl X509StoreBuilder {
    pub fn new() -> Result<X509StoreBuilder, ErrorStack> {
        unsafe {
            ffi::init();

            cvt_p(ffi::X509_STORE_new()).map(|p| X509StoreBuilder::from_ptr(p))
        }
    }

    #[must_use]
    pub fn build(self) -> X509Store {
        X509Store(ManuallyDrop::new(self).0)
    }
}

impl X509StoreBuilderRef {
    #[corresponds(X509_STORE_add_cert)]
    pub fn add_cert(&mut self, cert: impl AsRef<X509Ref>) -> Result<(), ErrorStack> {
        let cert = cert.as_ref();
        unsafe { cvt(ffi::X509_STORE_add_cert(self.as_ptr(), cert.as_ptr())) }
    }

    #[corresponds(X509_STORE_set_default_paths)]
    pub fn set_default_paths(&mut self) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::X509_STORE_set_default_paths(self.as_ptr())) }
    }

    #[corresponds(X509_STORE_set_flags)]
    pub fn try_set_flags(&mut self, flags: X509VerifyFlags) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::X509_STORE_set_flags(self.as_ptr(), flags.bits())) }
    }

    #[corresponds(X509_STORE_set_flags)]
    pub fn set_flags(&mut self, flags: X509VerifyFlags) {
        self.try_set_flags(flags).expect("use try_set_flags");
    }

    #[corresponds(X509_STORE_get0_param)]
    pub fn verify_param_mut(&mut self) -> &mut X509VerifyParamRef {
        unsafe { X509VerifyParamRef::from_ptr_mut(ffi::X509_STORE_get0_param(self.as_ptr())) }
    }

    #[corresponds(X509_STORE_set1_param)]
    pub fn set_param(&mut self, param: &X509VerifyParamRef) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::X509_STORE_set1_param(self.as_ptr(), param.as_ptr())) }
    }

    #[cfg(test)]
    pub fn objects_len(&self) -> usize {
        unsafe {
            StackRef::<X509Object>::from_ptr(ffi::X509_STORE_get0_objects(self.as_ptr())).len()
        }
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::X509_STORE;
    fn drop = ffi::X509_STORE_free;

    pub struct X509Store;
}

impl ToOwned for X509StoreRef {
    type Owned = X509Store;

    fn to_owned(&self) -> X509Store {
        unsafe {
            ffi::X509_STORE_up_ref(self.as_ptr());
            X509Store::from_ptr(self.as_ptr())
        }
    }
}

impl Clone for X509Store {
    fn clone(&self) -> X509Store {
        (**self).to_owned()
    }
}

impl X509StoreRef {
    #[deprecated(
        note = "This method is unsound https://github.com/sfackler/rust-openssl/issues/2096"
    )]
    #[corresponds(X509_STORE_get0_objects)]
    #[must_use]
    pub fn objects(&self) -> &StackRef<X509Object> {
        unsafe { StackRef::from_ptr(ffi::X509_STORE_get0_objects(self.as_ptr())) }
    }

    #[cfg(test)]
    #[allow(deprecated)]
    #[must_use]
    pub fn objects_len(&self) -> usize {
        self.objects().len()
    }
}

#[test]
#[allow(clippy::redundant_clone)]
#[should_panic = "Shared X509Store can't be mutated"]
fn set_cert_store_pevents_mutability() {
    use crate::ssl::*;

    let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
    let store = X509StoreBuilder::new().unwrap().build();

    ctx.set_cert_store(store.clone());

    let _aliased_store = ctx.cert_store_mut();
}
