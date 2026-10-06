use crate::bio::MemBioSlice;
use crate::error::ErrorStack;
use crate::ffi;
use crate::stack::StackRef;
use crate::x509::verify::{X509VerifyFlags, X509VerifyParamRef};
use crate::x509::{X509Object, X509Ref};
use crate::{cvt, cvt_p};
use foreign_types::{ForeignType, ForeignTypeRef};
use openssl_macros::corresponds;
use std::mem::ManuallyDrop;
use std::ptr;

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

    #[corresponds(PEM_X509_INFO_read_bio)]
    pub fn add_pem(&mut self, pem: &[u8]) -> Result<usize, ErrorStack> {
        unsafe {
            ffi::init();
            let bio = MemBioSlice::new(pem)?;
            let infos = cvt_p(ffi::PEM_X509_INFO_read_bio(
                bio.as_ptr(),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            ))?;
            let stack = infos.cast::<ffi::_STACK>();
            let mut added = Ok(0);
            for index in 0..ffi::sk_num(stack) {
                let info = ffi::sk_value(stack, index).cast::<ffi::X509_INFO>();
                added = added.and_then(|count| self.add_info(&*info).map(|more| count + more));
            }
            for index in 0..ffi::sk_num(stack) {
                ffi::X509_INFO_free(ffi::sk_value(stack, index).cast());
            }
            ffi::sk_free(stack);
            match added {
                Ok(0) => Err(ErrorStack::get()),
                other => other,
            }
        }
    }

    fn add_info(&mut self, info: &ffi::X509_INFO) -> Result<usize, ErrorStack> {
        let mut count = 0;
        if !info.x509.is_null() {
            // SAFETY: `info.x509` is a live certificate owned by the X509_INFO stack, and the store takes its own reference.
            cvt(unsafe { ffi::X509_STORE_add_cert(self.as_ptr(), info.x509) })?;
            count += 1;
        }
        if !info.crl.is_null() {
            // SAFETY: `info.crl` is a live CRL owned by the X509_INFO stack, and the store takes its own reference.
            cvt(unsafe { ffi::X509_STORE_add_crl(self.as_ptr(), info.crl) })?;
            count += 1;
        }
        Ok(count)
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
