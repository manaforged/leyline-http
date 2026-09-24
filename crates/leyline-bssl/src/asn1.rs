use crate::ffi;
use foreign_types::{ForeignType, ForeignTypeRef};
use libc::{c_int, c_long, time_t};
use std::cmp::Ordering;
use std::ffi::CString;
use std::fmt;
use std::ptr;
use std::slice;
use std::str;

use crate::bio::MemBio;
use crate::bn::{BigNum, BigNumRef};
use crate::error::ErrorStack;
use crate::nid::Nid;
use crate::stack::Stackable;
use crate::string::OpensslString;
use crate::{cvt, cvt_p};
use openssl_macros::corresponds;

foreign_type_and_impl_send_sync! {
    type CType = ffi::ASN1_GENERALIZEDTIME;
    fn drop = ffi::ASN1_GENERALIZEDTIME_free;

    pub struct Asn1GeneralizedTime;
}

impl fmt::Display for Asn1GeneralizedTimeRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let bio = MemBio::new().ok();
        let msg = bio
            .as_ref()
            .and_then(|mem_bio| unsafe {
                cvt(ffi::ASN1_GENERALIZEDTIME_print(
                    mem_bio.as_ptr(),
                    self.as_ptr(),
                ))
                .ok()?;
                str::from_utf8(mem_bio.get_buf()).ok()
            })
            .unwrap_or("error");
        f.write_str(msg)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Asn1Type(c_int);

#[allow(missing_docs)]
impl Asn1Type {
    pub const EOC: Asn1Type = Asn1Type(ffi::V_ASN1_EOC);

    pub const BOOLEAN: Asn1Type = Asn1Type(ffi::V_ASN1_BOOLEAN);

    pub const INTEGER: Asn1Type = Asn1Type(ffi::V_ASN1_INTEGER);

    pub const BIT_STRING: Asn1Type = Asn1Type(ffi::V_ASN1_BIT_STRING);

    pub const OCTET_STRING: Asn1Type = Asn1Type(ffi::V_ASN1_OCTET_STRING);

    pub const NULL: Asn1Type = Asn1Type(ffi::V_ASN1_NULL);

    pub const OBJECT: Asn1Type = Asn1Type(ffi::V_ASN1_OBJECT);

    pub const OBJECT_DESCRIPTOR: Asn1Type = Asn1Type(ffi::V_ASN1_OBJECT_DESCRIPTOR);

    pub const EXTERNAL: Asn1Type = Asn1Type(ffi::V_ASN1_EXTERNAL);

    pub const REAL: Asn1Type = Asn1Type(ffi::V_ASN1_REAL);

    pub const ENUMERATED: Asn1Type = Asn1Type(ffi::V_ASN1_ENUMERATED);

    pub const UTF8STRING: Asn1Type = Asn1Type(ffi::V_ASN1_UTF8STRING);

    pub const SEQUENCE: Asn1Type = Asn1Type(ffi::V_ASN1_SEQUENCE);

    pub const SET: Asn1Type = Asn1Type(ffi::V_ASN1_SET);

    pub const NUMERICSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_NUMERICSTRING);

    pub const PRINTABLESTRING: Asn1Type = Asn1Type(ffi::V_ASN1_PRINTABLESTRING);

    pub const T61STRING: Asn1Type = Asn1Type(ffi::V_ASN1_T61STRING);

    pub const TELETEXSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_TELETEXSTRING);

    pub const VIDEOTEXSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_VIDEOTEXSTRING);

    pub const IA5STRING: Asn1Type = Asn1Type(ffi::V_ASN1_IA5STRING);

    pub const UTCTIME: Asn1Type = Asn1Type(ffi::V_ASN1_UTCTIME);

    pub const GENERALIZEDTIME: Asn1Type = Asn1Type(ffi::V_ASN1_GENERALIZEDTIME);

    pub const GRAPHICSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_GRAPHICSTRING);

    pub const ISO64STRING: Asn1Type = Asn1Type(ffi::V_ASN1_ISO64STRING);

    pub const VISIBLESTRING: Asn1Type = Asn1Type(ffi::V_ASN1_VISIBLESTRING);

    pub const GENERALSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_GENERALSTRING);

    pub const UNIVERSALSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_UNIVERSALSTRING);

    pub const BMPSTRING: Asn1Type = Asn1Type(ffi::V_ASN1_BMPSTRING);

    #[must_use]
    pub fn from_raw(value: c_int) -> Self {
        Asn1Type(value)
    }

    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TimeDiff {
    pub days: c_int,
    pub secs: c_int,
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::ASN1_TIME;
    fn drop = ffi::ASN1_TIME_free;
    pub struct Asn1Time;
}

impl Asn1TimeRef {
    #[corresponds(ASN1_TIME_diff)]
    pub fn diff(&self, compare: &Self) -> Result<TimeDiff, ErrorStack> {
        let mut days = 0;
        let mut secs = 0;
        let other = compare.as_ptr();

        let err = unsafe { ffi::ASN1_TIME_diff(&mut days, &mut secs, self.as_ptr(), other) };

        match err {
            0 => Err(ErrorStack::get()),
            _ => Ok(TimeDiff { days, secs }),
        }
    }

    #[corresponds(ASN1_TIME_compare)]
    pub fn compare(&self, other: &Self) -> Result<Ordering, ErrorStack> {
        let d = self.diff(other)?;
        if d.days > 0 || d.secs > 0 {
            return Ok(Ordering::Less);
        }
        if d.days < 0 || d.secs < 0 {
            return Ok(Ordering::Greater);
        }

        Ok(Ordering::Equal)
    }
}

impl PartialEq for Asn1TimeRef {
    fn eq(&self, other: &Asn1TimeRef) -> bool {
        self.diff(other)
            .map(|t| t.days == 0 && t.secs == 0)
            .unwrap_or(false)
    }
}

impl PartialEq<Asn1Time> for Asn1TimeRef {
    fn eq(&self, other: &Asn1Time) -> bool {
        self.diff(other)
            .map(|t| t.days == 0 && t.secs == 0)
            .unwrap_or(false)
    }
}

impl PartialEq<Asn1Time> for &Asn1TimeRef {
    fn eq(&self, other: &Asn1Time) -> bool {
        self.diff(other)
            .map(|t| t.days == 0 && t.secs == 0)
            .unwrap_or(false)
    }
}

impl PartialOrd for Asn1TimeRef {
    fn partial_cmp(&self, other: &Asn1TimeRef) -> Option<Ordering> {
        self.compare(other).ok()
    }
}

impl PartialOrd<Asn1Time> for Asn1TimeRef {
    fn partial_cmp(&self, other: &Asn1Time) -> Option<Ordering> {
        self.compare(other).ok()
    }
}

impl PartialOrd<Asn1Time> for &Asn1TimeRef {
    fn partial_cmp(&self, other: &Asn1Time) -> Option<Ordering> {
        self.compare(other).ok()
    }
}

impl fmt::Display for Asn1TimeRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        unsafe {
            let mem_bio = match MemBio::new() {
                Err(_) => return f.write_str("error"),
                Ok(m) => m,
            };
            let print_result = cvt(ffi::ASN1_TIME_print(mem_bio.as_ptr(), self.as_ptr()));
            match print_result {
                Err(_) => f.write_str("error"),
                Ok(_) => f.write_str(str::from_utf8_unchecked(mem_bio.get_buf())),
            }
        }
    }
}

impl fmt::Debug for Asn1TimeRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Asn1Time {
    #[corresponds(ASN1_TIME_new)]
    fn new() -> Result<Asn1Time, ErrorStack> {
        ffi::init();

        unsafe {
            let handle = cvt_p(ffi::ASN1_TIME_new())?;
            Ok(Asn1Time::from_ptr(handle))
        }
    }

    #[corresponds(X509_gmtime_adj)]
    fn from_period(period: c_long) -> Result<Asn1Time, ErrorStack> {
        ffi::init();

        unsafe {
            let handle = cvt_p(ffi::X509_gmtime_adj(ptr::null_mut(), period))?;
            Ok(Asn1Time::from_ptr(handle))
        }
    }

    pub fn days_from_now(days: u32) -> Result<Asn1Time, ErrorStack> {
        Self::from_period((days * 60 * 60 * 24) as _)
    }

    #[corresponds(ASN1_TIME_set)]
    pub fn from_unix(time: time_t) -> Result<Asn1Time, ErrorStack> {
        ffi::init();

        unsafe {
            #[allow(clippy::useless_conversion)]
            let handle = cvt_p(ffi::ASN1_TIME_set(ptr::null_mut(), time.into()))?;
            Ok(Asn1Time::from_ptr(handle))
        }
    }

    #[corresponds(ASN1_TIME_set_string)]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Asn1Time, ErrorStack> {
        unsafe {
            let s = CString::new(s).map_err(ErrorStack::internal_error)?;

            let time = Asn1Time::new()?;
            cvt(ffi::ASN1_TIME_set_string(time.as_ptr(), s.as_ptr()))?;

            Ok(time)
        }
    }
}

impl PartialEq for Asn1Time {
    fn eq(&self, other: &Asn1Time) -> bool {
        self.diff(other)
            .map(|t| t.days == 0 && t.secs == 0)
            .unwrap_or(false)
    }
}

impl PartialEq<Asn1TimeRef> for Asn1Time {
    fn eq(&self, other: &Asn1TimeRef) -> bool {
        self.diff(other)
            .map(|t| t.days == 0 && t.secs == 0)
            .unwrap_or(false)
    }
}

impl<'a> PartialEq<&'a Asn1TimeRef> for Asn1Time {
    fn eq(&self, other: &&'a Asn1TimeRef) -> bool {
        self.diff(other)
            .map(|t| t.days == 0 && t.secs == 0)
            .unwrap_or(false)
    }
}

impl PartialOrd for Asn1Time {
    fn partial_cmp(&self, other: &Asn1Time) -> Option<Ordering> {
        self.compare(other).ok()
    }
}

impl PartialOrd<Asn1TimeRef> for Asn1Time {
    fn partial_cmp(&self, other: &Asn1TimeRef) -> Option<Ordering> {
        self.compare(other).ok()
    }
}

impl<'a> PartialOrd<&'a Asn1TimeRef> for Asn1Time {
    fn partial_cmp(&self, other: &&'a Asn1TimeRef) -> Option<Ordering> {
        self.compare(other).ok()
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::ASN1_STRING;
    fn drop = ffi::ASN1_STRING_free;
    pub struct Asn1String;
}

impl Asn1StringRef {
    #[corresponds(ASN1_STRING_to_UTF8)]
    pub fn as_utf8(&self) -> Result<OpensslString, ErrorStack> {
        unsafe {
            let mut ptr = ptr::null_mut();
            let len = ffi::ASN1_STRING_to_UTF8(&mut ptr, self.as_ptr());
            if len < 0 {
                return Err(ErrorStack::get());
            }

            Ok(OpensslString::from_ptr(ptr.cast()))
        }
    }

    #[corresponds(ASN1_STRING_get0_data)]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(ASN1_STRING_get0_data(self.as_ptr()), self.len()) }
    }

    #[corresponds(ASN1_STRING_length)]
    #[must_use]
    pub fn len(&self) -> usize {
        unsafe { ffi::ASN1_STRING_length(self.as_ptr()) as usize }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for Asn1StringRef {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self.as_utf8() {
            Ok(openssl_string) => openssl_string.fmt(fmt),
            Err(_) => fmt.write_str("error"),
        }
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::ASN1_INTEGER;
    fn drop = ffi::ASN1_INTEGER_free;

    pub struct Asn1Integer;
}

impl Asn1Integer {
    pub fn from_bn(bn: &BigNumRef) -> Result<Self, ErrorStack> {
        bn.to_asn1_integer()
    }
}

impl Asn1IntegerRef {
    #[allow(clippy::unnecessary_cast)]
    #[allow(missing_docs)]
    #[deprecated(since = "0.10.6", note = "use to_bn instead")]
    #[must_use]
    pub fn get(&self) -> i64 {
        unsafe { crate::ffi::ASN1_INTEGER_get(self.as_ptr()) as i64 }
    }

    #[corresponds(ASN1_INTEGER_to_BN)]
    pub fn to_bn(&self) -> Result<BigNum, ErrorStack> {
        unsafe {
            cvt_p(crate::ffi::ASN1_INTEGER_to_BN(
                self.as_ptr(),
                ptr::null_mut(),
            ))
            .map(|p| BigNum::from_ptr(p))
        }
    }

    #[corresponds(ASN1_INTEGER_set)]
    pub fn set(&mut self, value: i32) -> Result<(), ErrorStack> {
        unsafe {
            cvt(crate::ffi::ASN1_INTEGER_set(
                self.as_ptr(),
                c_long::from(value),
            ))
        }
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::ASN1_BIT_STRING;
    fn drop = ffi::ASN1_BIT_STRING_free;
    pub struct Asn1BitString;
}

impl Asn1BitStringRef {
    #[corresponds(ASN1_STRING_get0_data)]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            let ptr = ASN1_STRING_get0_data(self.as_ptr().cast());
            if ptr.is_null() {
                return &[];
            }
            slice::from_raw_parts(ptr, self.len())
        }
    }

    #[corresponds(ASN1_STRING_get0_data)]
    #[must_use]
    pub fn to_str(&self) -> Option<&str> {
        str::from_utf8(self.as_slice()).ok()
    }

    #[corresponds(ASN1_STRING_length)]
    #[must_use]
    pub fn len(&self) -> usize {
        unsafe { ffi::ASN1_STRING_length(self.as_ptr().cast_const()) as usize }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::ASN1_OBJECT;
    fn drop = ffi::ASN1_OBJECT_free;

    pub struct Asn1Object;
}

impl Stackable for Asn1Object {
    type StackType = ffi::stack_st_ASN1_OBJECT;
}

impl Asn1Object {
    #[corresponds(OBJ_txt2obj)]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(txt: &str) -> Result<Asn1Object, ErrorStack> {
        unsafe {
            ffi::init();
            let txt = CString::new(txt).map_err(ErrorStack::internal_error)?;
            let obj: *mut ffi::ASN1_OBJECT = cvt_p(ffi::OBJ_txt2obj(txt.as_ptr(), 0))?;
            Ok(Asn1Object::from_ptr(obj))
        }
    }
}

impl Asn1ObjectRef {
    #[must_use]
    pub fn nid(&self) -> Nid {
        unsafe { Nid::from_raw(ffi::OBJ_obj2nid(self.as_ptr())) }
    }
}

impl fmt::Display for Asn1ObjectRef {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        unsafe {
            let mut buf = [0u8; 80];
            let len = ffi::OBJ_obj2txt(
                buf.as_mut_ptr().cast(),
                buf.len() as c_int,
                self.as_ptr(),
                0,
            );
            fmt.write_str(
                buf.get(..len as usize)
                    .and_then(|s| str::from_utf8(s).ok())
                    .unwrap_or("error"),
            )
        }
    }
}

impl fmt::Debug for Asn1ObjectRef {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, fmt)
    }
}

use crate::ffi::ASN1_STRING_get0_data;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::bn::BigNum;
    use crate::nid::Nid;

    #[test]
    fn bn_cvt() {
        fn roundtrip(bn: BigNum) {
            let large = Asn1Integer::from_bn(&bn).unwrap();
            assert_eq!(large.to_bn().unwrap(), bn);
        }

        roundtrip(BigNum::from_dec_str("1000000000000000000000000000000000").unwrap());
        roundtrip(-BigNum::from_dec_str("1000000000000000000000000000000000").unwrap());
        roundtrip(BigNum::from_u32(1234).unwrap());
        roundtrip(-BigNum::from_u32(1234).unwrap());
    }

    #[test]
    fn time_from_str() {
        Asn1Time::from_str("99991231235959Z").unwrap();
    }

    #[test]
    fn time_from_unix() {
        let t = Asn1Time::from_unix(0).unwrap();
        assert_eq!("Jan  1 00:00:00 1970 GMT", t.to_string());
    }

    #[test]
    fn time_eq() {
        let a = Asn1Time::from_str("99991231235959Z").unwrap();
        let b = Asn1Time::from_str("99991231235959Z").unwrap();
        let c = Asn1Time::from_str("99991231235958Z").unwrap();
        let a_ref = a.as_ref();
        let b_ref = b.as_ref();
        let c_ref = c.as_ref();
        assert!(a == b);
        assert!(a != c);
        assert!(a == b_ref);
        assert!(a != c_ref);
        assert!(b_ref == a);
        assert!(c_ref != a);
        assert!(a_ref == b_ref);
        assert!(a_ref != c_ref);
    }

    #[test]
    fn time_ord() {
        let a = Asn1Time::from_str("99991231235959Z").unwrap();
        let b = Asn1Time::from_str("99991231235959Z").unwrap();
        let c = Asn1Time::from_str("99991231235958Z").unwrap();
        let a_ref = a.as_ref();
        let b_ref = b.as_ref();
        let c_ref = c.as_ref();
        assert!(a >= b);
        assert!(a > c);
        assert!(b <= a);
        assert!(c < a);

        assert!(a_ref >= b);
        assert!(a_ref > c);
        assert!(b_ref <= a);
        assert!(c_ref < a);

        assert!(a >= b_ref);
        assert!(a > c_ref);
        assert!(b <= a_ref);
        assert!(c < a_ref);

        assert!(a_ref >= b_ref);
        assert!(a_ref > c_ref);
        assert!(b_ref <= a_ref);
        assert!(c_ref < a_ref);
    }

    #[test]
    fn object_from_str() {
        let object = Asn1Object::from_str("2.16.840.1.101.3.4.2.1").unwrap();
        assert_eq!(object.nid(), Nid::SHA256);
    }

    #[test]
    fn object_from_str_with_invalid_input() {
        Asn1Object::from_str("NOT AN OID")
            .map(|object| object.to_string())
            .expect_err("parsing invalid OID should fail");
    }
}
