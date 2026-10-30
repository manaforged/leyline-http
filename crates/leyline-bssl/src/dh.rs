use crate::ffi;
use foreign_types::ForeignTypeRef;
use openssl_macros::corresponds;
use std::ptr;

use crate::pkey::{HasParams, Params};
use crate::{cvt, cvt_p};

generic_foreign_type_and_impl_send_sync! {
    type CType = ffi::DH;
    fn drop = ffi::DH_free;

    pub struct Dh<T>;

    pub struct DhRef<T>;
}

impl<T> DhRef<T>
where
    T: HasParams,
{
    to_pem! {
        #[corresponds(PEM_write_bio_DHparams)]
        params_to_pem,
        ffi::PEM_write_bio_DHparams
    }

    to_der! {
        #[corresponds(i2d_DHparams)]
        params_to_der,
        ffi::i2d_DHparams
    }
}

impl Dh<Params> {
    from_pem! {
        #[corresponds(PEM_read_bio_DHparams)]
        params_from_pem,
        Dh<Params>,
        ffi::PEM_read_bio_DHparams
    }

    from_der! {
        #[corresponds(d2i_DHparams)]
        params_from_der,
        Dh<Params>,
        ffi::d2i_DHparams,
        ::libc::c_long
    }
}

#[cfg(test)]
mod tests {

    use crate::dh::Dh;
    use crate::ssl::{SslContext, SslMethod};

    #[test]
    fn test_dh_from_pem() {
        let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
        let params = include_bytes!("../test/dhparams.pem");
        let dh = Dh::params_from_pem(params).unwrap();
        ctx.set_tmp_dh(&dh).unwrap();
    }

    #[test]
    fn test_dh_from_der() {
        let params = include_bytes!("../test/dhparams.pem");
        let dh = Dh::params_from_pem(params).unwrap();
        let der = dh.params_to_der().unwrap();
        Dh::params_from_der(&der).unwrap();
    }
}
