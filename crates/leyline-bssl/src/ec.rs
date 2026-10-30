use foreign_types::{ForeignType, ForeignTypeRef};
use openssl_macros::corresponds;
use std::fmt;
use std::ptr;

use crate::bn::{BigNumContextRef, BigNumRef};
use crate::error::ErrorStack;
use crate::ffi;
use crate::nid::Nid;
use crate::pkey::{HasParams, HasPrivate, HasPublic, Params, Private, Public};
use crate::try_int;
use crate::{cvt, cvt_n, cvt_p, init};

#[derive(Copy, Clone)]
pub struct PointConversionForm(ffi::point_conversion_form_t);

foreign_type_and_impl_send_sync! {
    type CType = ffi::EC_GROUP;
    fn drop = ffi::EC_GROUP_free;

    pub struct EcGroup;
}

impl EcGroup {
    #[corresponds(EC_GROUP_new)]
    pub fn from_curve_name(nid: Nid) -> Result<EcGroup, ErrorStack> {
        unsafe {
            init();
            cvt_p(ffi::EC_GROUP_new_by_curve_name(nid.as_raw())).map(|p| EcGroup::from_ptr(p))
        }
    }
}

impl EcGroupRef {
    #[corresponds(EC_GROUP_get_degree)]
    #[allow(clippy::unnecessary_cast)]
    #[must_use]
    pub fn degree(&self) -> u32 {
        unsafe { ffi::EC_GROUP_get_degree(self.as_ptr()) as u32 }
    }

    #[corresponds(EC_GROUP_get_order)]
    pub fn order(
        &self,
        order: &mut BigNumRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::EC_GROUP_get_order(
                self.as_ptr(),
                order.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    #[corresponds(EC_GROUP_get_curve_name)]
    #[must_use]
    pub fn curve_name(&self) -> Option<Nid> {
        let nid = unsafe { ffi::EC_GROUP_get_curve_name(self.as_ptr()) };
        if nid > 0 {
            Some(Nid::from_raw(nid))
        } else {
            None
        }
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::EC_POINT;
    fn drop = ffi::EC_POINT_free;

    pub struct EcPoint;
}

impl EcPointRef {
    #[corresponds(EC_POINT_add)]
    pub fn add(
        &mut self,
        group: &EcGroupRef,
        a: &EcPointRef,
        b: &EcPointRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::EC_POINT_add(
                group.as_ptr(),
                self.as_ptr(),
                a.as_ptr(),
                b.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    #[corresponds(EC_POINT_mul)]
    pub fn mul(
        &mut self,
        group: &EcGroupRef,
        q: &EcPointRef,
        m: &BigNumRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::EC_POINT_mul(
                group.as_ptr(),
                self.as_ptr(),
                ptr::null(),
                q.as_ptr(),
                m.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    #[corresponds(EC_POINT_point2oct)]
    pub fn to_bytes(
        &self,
        group: &EcGroupRef,
        form: PointConversionForm,
        ctx: &mut BigNumContextRef,
    ) -> Result<Vec<u8>, ErrorStack> {
        unsafe {
            let len = ffi::EC_POINT_point2oct(
                group.as_ptr(),
                self.as_ptr(),
                form.0,
                ptr::null_mut(),
                0,
                ctx.as_ptr(),
            );
            if len == 0 {
                return Err(ErrorStack::get());
            }
            let mut buf = vec![0; len];
            let len = ffi::EC_POINT_point2oct(
                group.as_ptr(),
                self.as_ptr(),
                form.0,
                buf.as_mut_ptr(),
                len,
                ctx.as_ptr(),
            );
            if len == 0 {
                Err(ErrorStack::get())
            } else {
                Ok(buf)
            }
        }
    }

    #[corresponds(EC_POINT_dup)]
    pub fn to_owned(&self, group: &EcGroupRef) -> Result<EcPoint, ErrorStack> {
        unsafe {
            cvt_p(ffi::EC_POINT_dup(self.as_ptr(), group.as_ptr())).map(|p| EcPoint::from_ptr(p))
        }
    }

    pub fn eq(
        &self,
        group: &EcGroupRef,
        other: &EcPointRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<bool, ErrorStack> {
        unsafe {
            let res = cvt_n(ffi::EC_POINT_cmp(
                group.as_ptr(),
                self.as_ptr(),
                other.as_ptr(),
                ctx.as_ptr(),
            ))?;
            Ok(res == 0)
        }
    }
}

impl EcPoint {
    #[corresponds(EC_POINT_new)]
    pub fn new(group: &EcGroupRef) -> Result<EcPoint, ErrorStack> {
        unsafe { cvt_p(ffi::EC_POINT_new(group.as_ptr())).map(|p| EcPoint::from_ptr(p)) }
    }

    #[corresponds(EC_POINT_oct2point)]
    pub fn from_bytes(
        group: &EcGroupRef,
        buf: &[u8],
        ctx: &mut BigNumContextRef,
    ) -> Result<EcPoint, ErrorStack> {
        let point = EcPoint::new(group)?;
        unsafe {
            cvt(ffi::EC_POINT_oct2point(
                group.as_ptr(),
                point.as_ptr(),
                buf.as_ptr(),
                buf.len(),
                ctx.as_ptr(),
            ))?;
        }
        Ok(point)
    }
}

generic_foreign_type_and_impl_send_sync! {
    type CType = ffi::EC_KEY;
    fn drop = ffi::EC_KEY_free;

    pub struct EcKey<T>;

    pub struct EcKeyRef<T>;
}

impl<T> EcKeyRef<T>
where
    T: HasPrivate,
{
    private_key_to_pem! {
        #[corresponds(PEM_write_bio_ECPrivateKey)]
        private_key_to_pem,
        #[corresponds(PEM_write_bio_ECPrivateKey)]
        private_key_to_pem_passphrase,
        ffi::PEM_write_bio_ECPrivateKey
    }

    to_der! {
        #[corresponds(i2d_ECPrivateKey)]
        private_key_to_der,
        ffi::i2d_ECPrivateKey
    }
}

impl<T> EcKeyRef<T>
where
    T: HasPublic,
{
    to_pem! {
        #[corresponds(PEM_write_bio_EC_PUBKEY)]
        public_key_to_pem,
        ffi::PEM_write_bio_EC_PUBKEY
    }

    to_der! {
        #[corresponds(i2d_EC_PUBKEY)]
        public_key_to_der,
        ffi::i2d_EC_PUBKEY
    }
}

impl<T> EcKeyRef<T>
where
    T: HasParams,
{
    #[corresponds(EC_KEY_get0_group)]
    #[must_use]
    pub fn group(&self) -> &EcGroupRef {
        unsafe {
            let ptr = ffi::EC_KEY_get0_group(self.as_ptr());
            EcGroupRef::from_ptr(ptr.cast_mut())
        }
    }
}

impl<T> ToOwned for EcKeyRef<T> {
    type Owned = EcKey<T>;

    fn to_owned(&self) -> EcKey<T> {
        unsafe {
            let r = ffi::EC_KEY_up_ref(self.as_ptr());
            assert!(r == 1);
            EcKey::from_ptr(self.as_ptr())
        }
    }
}

impl EcKey<Params> {
    #[corresponds(EC_KEY_new_by_curve_name)]
    pub fn from_curve_name(nid: Nid) -> Result<EcKey<Params>, ErrorStack> {
        unsafe {
            init();
            cvt_p(ffi::EC_KEY_new_by_curve_name(nid.as_raw())).map(|p| EcKey::from_ptr(p))
        }
    }
}

impl EcKey<Public> {
    from_pem! {
        #[corresponds(PEM_read_bio_EC_PUBKEY)]
        public_key_from_pem,
        EcKey<Public>,
        ffi::PEM_read_bio_EC_PUBKEY
    }

    from_der! {
        #[corresponds(d2i_EC_PUBKEY)]
        public_key_from_der,
        EcKey<Public>,
        ffi::d2i_EC_PUBKEY,
        ::libc::c_long
    }
}

impl EcKey<Private> {
    pub fn generate(group: &EcGroupRef) -> Result<EcKey<Private>, ErrorStack> {
        unsafe {
            cvt_p(ffi::EC_KEY_new())
                .map(|p| EcKey::from_ptr(p))
                .and_then(|key| {
                    cvt(ffi::EC_KEY_set_group(key.as_ptr(), group.as_ptr())).map(|_| key)
                })
                .and_then(|key| cvt(ffi::EC_KEY_generate_key(key.as_ptr())).map(|_| key))
        }
    }

    private_key_from_pem! {
        #[corresponds(PEM_read_bio_ECPrivateKey)]
        private_key_from_pem,

        #[corresponds(PEM_read_bio_ECPrivateKey)]
        private_key_from_pem_passphrase,

        #[corresponds(PEM_read_bio_ECPrivateKey)]
        private_key_from_pem_callback,
        EcKey<Private>,
        ffi::PEM_read_bio_ECPrivateKey
    }

    from_der! {
        #[corresponds(d2i_ECPrivateKey)]
        private_key_from_der,
        EcKey<Private>,
        ffi::d2i_ECPrivateKey,
        ::libc::c_long
    }
}

impl<T> Clone for EcKey<T> {
    fn clone(&self) -> EcKey<T> {
        (**self).to_owned()
    }
}

impl<T> fmt::Debug for EcKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "EcKey")
    }
}

#[cfg(test)]
mod test {

    use super::*;

    use crate::nid::Nid;

    #[test]
    fn key_new_by_curve_name() {
        EcKey::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    }

    #[test]
    fn generate() {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        EcKey::generate(&group).unwrap();
    }

    #[test]
    #[allow(clippy::redundant_clone)]
    fn dup() {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = EcKey::generate(&group).unwrap();
        drop(key.clone());
    }

    #[test]
    fn point_new() {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        EcPoint::new(&group).unwrap();
    }
}
