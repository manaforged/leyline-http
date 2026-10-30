//! BigNum implementation
//!
//! Large numbers are important for a cryptographic library.  OpenSSL implementation
//! of BigNum uses dynamically assigned memory to store an array of bit chunks.  This
//! allows numbers of any size to be compared and mathematical functions performed.
//!
//! OpenSSL wiki describes the [`BIGNUM`] data structure.
//!
//! [`BIGNUM`]: https://wiki.openssl.org/index.php/Manual:Bn_internal(3)
use crate::ffi;
use foreign_types::{ForeignType, ForeignTypeRef};
use libc::c_int;
use std::cmp::Ordering;
use std::ops::{Add, Deref, Div, Mul, Neg, Rem, Shl, Shr, Sub};
use std::{fmt, ptr};

use crate::asn1::Asn1Integer;
use crate::error::ErrorStack;
use crate::ffi::BN_is_negative;
use crate::string::OpensslString;
use crate::{cvt, cvt_p};
use openssl_macros::corresponds;

/// Options for the most significant bits of a randomly generated `BigNum`.
pub struct MsbOption(c_int);

impl MsbOption {
    /// The most significant bit of the number may be 0.
    pub const MAYBE_ZERO: MsbOption = MsbOption(-1);

    /// The most significant bit of the number must be 1.
    pub const ONE: MsbOption = MsbOption(0);
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::BN_CTX;
    fn drop = ffi::BN_CTX_free;

    /// Temporary storage for BigNums on the secure heap
    ///
    /// BigNum values are stored dynamically and therefore can be expensive
    /// to allocate.  BigNumContext and the OpenSSL [`BN_CTX`] structure are used
    /// internally when passing BigNum values between subroutines.
    ///
    /// [`BN_CTX`]: https://www.openssl.org/docs/man1.1.0/crypto/BN_CTX_new.html
    pub struct BigNumContext;
}

impl BigNumContext {
    /// Returns a new `BigNumContext`.
    #[corresponds(BN_CTX_new)]
    pub fn new() -> Result<BigNumContext, ErrorStack> {
        unsafe {
            ffi::init();
            cvt_p(ffi::BN_CTX_new()).map(|p| BigNumContext::from_ptr(p))
        }
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::BIGNUM;
    fn drop = ffi::BN_free;

    /// Dynamically sized large number impelementation
    ///
    /// Perform large number mathematics.  Create a new BigNum
    /// with [`new`].  Perform standard mathematics on large numbers using
    /// methods from [`Dref<Target = BigNumRef>`]
    ///
    /// OpenSSL documentation at [`BN_new`].
    ///
    /// [`new`]: struct.BigNum.html#method.new
    /// [`Dref<Target = BigNumRef>`]: struct.BigNum.html#deref-methods
    /// [`BN_new`]: https://www.openssl.org/docs/man1.1.0/crypto/BN_new.html
    ///
    /// # Examples
    /// ```
    /// use leyline_bssl::bn::BigNum;
    /// # use leyline_bssl::error::ErrorStack;
    /// # fn bignums() -> Result< (), ErrorStack > {
    /// let little_big = BigNum::from_u32(std::u32::MAX)?;
    /// assert_eq!(*&little_big.num_bytes(), 4);
    /// # Ok(())
    /// # }
    /// # fn main () { bignums(); }
    /// ```
    pub struct BigNum;
}

impl BigNumRef {
    /// Erases the memory used by this `BigNum`, resetting its value to 0.
    ///
    /// This can be used to destroy sensitive data such as keys when they are no longer needed.
    #[corresponds(BN_clear)]
    pub fn clear(&mut self) {
        unsafe { ffi::BN_clear(self.as_ptr()) }
    }

    /// Places `a + b` in `self`.  [`core::ops::Add`] is also implemented for `BigNumRef`.
    ///
    /// [`core::ops::Add`]: struct.BigNumRef.html#method.add
    #[corresponds(BN_add)]
    pub fn checked_add(&mut self, a: &BigNumRef, b: &BigNumRef) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::BN_add(self.as_ptr(), a.as_ptr(), b.as_ptr())) }
    }

    /// Places `a - b` in `self`. [`core::ops::Sub`] is also implemented for `BigNumRef`.
    ///
    /// [`core::ops::Sub`]: struct.BigNumRef.html#method.sub
    #[corresponds(BN_sub)]
    pub fn checked_sub(&mut self, a: &BigNumRef, b: &BigNumRef) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::BN_sub(self.as_ptr(), a.as_ptr(), b.as_ptr())) }
    }

    /// Places `a << n` in `self`.  Equivalent to `a * 2 ^ n`.
    #[corresponds(BN_lshift)]
    #[allow(clippy::useless_conversion)]
    pub fn lshift(&mut self, a: &BigNumRef, n: i32) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::BN_lshift(self.as_ptr(), a.as_ptr(), n.into())) }
    }

    /// Places `a >> n` in `self`. Equivalent to `a / 2 ^ n`.
    #[corresponds(BN_rshift)]
    #[allow(clippy::useless_conversion)]
    pub fn rshift(&mut self, a: &BigNumRef, n: i32) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::BN_rshift(self.as_ptr(), a.as_ptr(), n.into())) }
    }

    /// Creates a new BigNum with the same value.
    #[corresponds(BN_dup)]
    pub fn to_owned(&self) -> Result<BigNum, ErrorStack> {
        unsafe { cvt_p(ffi::BN_dup(self.as_ptr())).map(|b| BigNum::from_ptr(b)) }
    }

    /// Sets the sign of `self`.  Pass true to set `self` to a negative.  False sets
    /// `self` positive.
    #[corresponds(BN_set_negative)]
    pub fn set_negative(&mut self, negative: bool) {
        unsafe { ffi::BN_set_negative(self.as_ptr(), c_int::from(negative)) }
    }

    /// Returns `true` if `self` is negative.
    #[corresponds(BN_is_negative)]
    #[must_use]
    pub fn is_negative(&self) -> bool {
        unsafe { BN_is_negative(self.as_ptr()) == 1 }
    }

    /// Returns the number of significant bits in `self`.
    #[corresponds(BN_num_bits)]
    #[must_use]
    pub fn num_bits(&self) -> i32 {
        unsafe { ffi::BN_num_bits(self.as_ptr()) as i32 }
    }

    /// Returns the size of `self` in bytes. Implemented natively.
    #[must_use]
    pub fn num_bytes(&self) -> i32 {
        (self.num_bits() + 7) / 8
    }

    /// Generates a cryptographically strong pseudo-random `BigNum`, placing it in `self`.
    ///
    /// # Parameters
    ///
    /// * `bits`: Length of the number in bits.
    /// * `msb`: The desired properties of the most significant bit. See [`constants`].
    /// * `odd`: If `true`, the generated number will be odd.
    ///
    /// # Examples
    ///
    /// ```
    /// use leyline_bssl::bn::{BigNum, MsbOption};
    /// use leyline_bssl::error::ErrorStack;
    ///
    /// fn generate_random() -> Result< BigNum, ErrorStack > {
    ///    let mut big = BigNum::new()?;
    ///
    ///    // Generates a 128-bit odd random number
    ///    big.rand(128, MsbOption::MAYBE_ZERO, true);
    ///    Ok((big))
    /// }
    /// ```
    ///
    /// [`constants`]: index.html#constants
    #[corresponds(BN_rand)]
    #[allow(clippy::useless_conversion)]
    pub fn rand(&mut self, bits: i32, msb: MsbOption, odd: bool) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::BN_rand(
                self.as_ptr(),
                bits.into(),
                msb.0,
                c_int::from(odd),
            ))
        }
    }

    /// Places the result of `a * b` in `self`.
    /// [`core::ops::Mul`] is also implemented for `BigNumRef`.
    ///
    /// [`core::ops::Mul`]: struct.BigNumRef.html#method.mul
    #[corresponds(BN_mul)]
    pub fn checked_mul(
        &mut self,
        a: &BigNumRef,
        b: &BigNumRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::BN_mul(
                self.as_ptr(),
                a.as_ptr(),
                b.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    /// Places the result of `a / b` in `self`. The remainder is discarded.
    /// [`core::ops::Div`] is also implemented for `BigNumRef`.
    ///
    /// [`core::ops::Div`]: struct.BigNumRef.html#method.div
    #[corresponds(BN_div)]
    pub fn checked_div(
        &mut self,
        a: &BigNumRef,
        b: &BigNumRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::BN_div(
                self.as_ptr(),
                ptr::null_mut(),
                a.as_ptr(),
                b.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    /// Places the result of `a % b` in `self`.
    #[corresponds(BN_div)]
    pub fn checked_rem(
        &mut self,
        a: &BigNumRef,
        b: &BigNumRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::BN_div(
                ptr::null_mut(),
                self.as_ptr(),
                a.as_ptr(),
                b.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    /// Places the result of `a^p` in `self`.
    #[corresponds(BN_exp)]
    pub fn exp(
        &mut self,
        a: &BigNumRef,
        p: &BigNumRef,
        ctx: &mut BigNumContextRef,
    ) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::BN_exp(
                self.as_ptr(),
                a.as_ptr(),
                p.as_ptr(),
                ctx.as_ptr(),
            ))
        }
    }

    /// Returns a big-endian byte vector representation of the absolute value of `self`.
    ///
    /// `self` can be recreated by using `from_slice`.
    ///
    /// ```
    /// # use leyline_bssl::bn::BigNum;
    /// let s = -BigNum::from_u32(4543).unwrap();
    /// let r = BigNum::from_u32(4543).unwrap();
    ///
    /// let s_vec = s.to_vec();
    /// assert_eq!(BigNum::from_slice(&s_vec).unwrap(), r);
    /// ```
    #[corresponds(BN_bn2bin)]
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        let size = self.num_bytes() as usize;
        let mut v = Vec::with_capacity(size);
        unsafe {
            ffi::BN_bn2bin(self.as_ptr(), v.as_mut_ptr());
            v.set_len(size);
        }
        v
    }

    /// Returns a decimal string representation of `self`.
    ///
    /// ```
    /// # use leyline_bssl::bn::BigNum;
    /// let s = -BigNum::from_u32(12345).unwrap();
    ///
    /// assert_eq!(&**s.to_dec_str().unwrap(), "-12345");
    /// ```
    #[corresponds(BN_bn2dec)]
    pub fn to_dec_str(&self) -> Result<OpensslString, ErrorStack> {
        unsafe {
            let buf = cvt_p(ffi::BN_bn2dec(self.as_ptr()))?;
            Ok(OpensslString::from_ptr(buf))
        }
    }

    /// Returns a hexadecimal string representation of `self`.
    ///
    /// ```
    /// # use leyline_bssl::bn::BigNum;
    /// let s = -BigNum::from_u32(0x99ff).unwrap();
    ///
    /// assert_eq!(&**s.to_hex_str().unwrap(), "-99ff");
    /// ```
    #[corresponds(BN_bn2hex)]
    pub fn to_hex_str(&self) -> Result<OpensslString, ErrorStack> {
        unsafe {
            let buf = cvt_p(ffi::BN_bn2hex(self.as_ptr()))?;
            Ok(OpensslString::from_ptr(buf))
        }
    }

    /// Returns an `Asn1Integer` containing the value of `self`.
    #[corresponds(BN_to_ASN1_INTEGER)]
    pub fn to_asn1_integer(&self) -> Result<Asn1Integer, ErrorStack> {
        unsafe {
            cvt_p(ffi::BN_to_ASN1_INTEGER(self.as_ptr(), ptr::null_mut()))
                .map(|p| Asn1Integer::from_ptr(p))
        }
    }
}

impl BigNum {
    /// Creates a new `BigNum` with the value 0.
    #[corresponds(BN_new)]
    pub fn new() -> Result<BigNum, ErrorStack> {
        unsafe {
            ffi::init();
            let v = cvt_p(ffi::BN_new())?;
            Ok(BigNum::from_ptr(v))
        }
    }

    /// Creates a new `BigNum` with the given value.
    #[corresponds(BN_set_word)]
    pub fn from_u32(n: u32) -> Result<BigNum, ErrorStack> {
        BigNum::new().and_then(|v| unsafe {
            cvt(ffi::BN_set_word(v.as_ptr(), ffi::BN_ULONG::from(n))).map(|_| v)
        })
    }

    /// Creates a new `BigNum` from an unsigned, big-endian encoded number of arbitrary length.
    ///
    /// OpenSSL documentation at [`BN_bin2bn`]
    ///
    /// [`BN_bin2bn`]: https://www.openssl.org/docs/man1.1.0/crypto/BN_bin2bn.html
    ///
    /// ```
    /// # use leyline_bssl::bn::BigNum;
    /// let bignum = BigNum::from_slice(&[0x12, 0x00, 0x34]).unwrap();
    ///
    /// assert_eq!(bignum, BigNum::from_u32(0x120034).unwrap());
    /// ```
    #[corresponds(BN_bin2bn)]
    pub fn from_slice(n: &[u8]) -> Result<BigNum, ErrorStack> {
        unsafe {
            ffi::init();
            assert!(n.len() <= c_int::MAX as usize);
            cvt_p(ffi::BN_bin2bn(n.as_ptr(), n.len(), ptr::null_mut())).map(|p| BigNum::from_ptr(p))
        }
    }
}

impl fmt::Debug for BigNumRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.to_dec_str() {
            Ok(s) => f.write_str(&s),
            Err(e) => Err(e.into()),
        }
    }
}

impl fmt::Debug for BigNum {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.to_dec_str() {
            Ok(s) => f.write_str(&s),
            Err(e) => Err(e.into()),
        }
    }
}

impl fmt::Display for BigNumRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.to_dec_str() {
            Ok(s) => f.write_str(&s),
            Err(e) => Err(e.into()),
        }
    }
}

impl fmt::Display for BigNum {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.to_dec_str() {
            Ok(s) => f.write_str(&s),
            Err(e) => Err(e.into()),
        }
    }
}

impl PartialEq<BigNumRef> for BigNumRef {
    fn eq(&self, oth: &BigNumRef) -> bool {
        self.cmp(oth) == Ordering::Equal
    }
}

impl PartialEq<BigNum> for BigNumRef {
    fn eq(&self, oth: &BigNum) -> bool {
        self.eq(&**oth)
    }
}

impl Eq for BigNumRef {}

impl PartialEq for BigNum {
    fn eq(&self, oth: &BigNum) -> bool {
        self.deref().eq(oth)
    }
}

impl PartialEq<BigNumRef> for BigNum {
    fn eq(&self, oth: &BigNumRef) -> bool {
        self.deref().eq(oth)
    }
}

impl Eq for BigNum {}

impl PartialOrd<BigNumRef> for BigNumRef {
    fn partial_cmp(&self, oth: &BigNumRef) -> Option<Ordering> {
        Some(self.cmp(oth))
    }
}

impl PartialOrd<BigNum> for BigNumRef {
    fn partial_cmp(&self, oth: &BigNum) -> Option<Ordering> {
        Some(self.cmp(&**oth))
    }
}

impl Ord for BigNumRef {
    fn cmp(&self, oth: &BigNumRef) -> Ordering {
        unsafe { ffi::BN_cmp(self.as_ptr(), oth.as_ptr()).cmp(&0) }
    }
}

impl PartialOrd for BigNum {
    fn partial_cmp(&self, oth: &BigNum) -> Option<Ordering> {
        Some(self.cmp(oth))
    }
}

impl PartialOrd<BigNumRef> for BigNum {
    fn partial_cmp(&self, oth: &BigNumRef) -> Option<Ordering> {
        self.deref().partial_cmp(oth)
    }
}

impl Ord for BigNum {
    fn cmp(&self, oth: &BigNum) -> Ordering {
        self.deref().cmp(&**oth)
    }
}

macro_rules! delegate {
    ($t:ident, $m:ident) => {
        impl<'a, 'b> $t<&'b BigNum> for &'a BigNumRef {
            type Output = BigNum;

            fn $m(self, oth: &BigNum) -> BigNum {
                $t::$m(self, oth.deref())
            }
        }

        impl<'a, 'b> $t<&'b BigNumRef> for &'a BigNum {
            type Output = BigNum;

            fn $m(self, oth: &BigNumRef) -> BigNum {
                $t::$m(self.deref(), oth)
            }
        }

        impl<'a, 'b> $t<&'b BigNum> for &'a BigNum {
            type Output = BigNum;

            fn $m(self, oth: &BigNum) -> BigNum {
                $t::$m(self.deref(), oth.deref())
            }
        }
    };
}

impl Add<&BigNumRef> for &BigNumRef {
    type Output = BigNum;

    fn add(self, oth: &BigNumRef) -> BigNum {
        let mut r = BigNum::new().unwrap();
        r.checked_add(self, oth).unwrap();
        r
    }
}

delegate!(Add, add);

impl Sub<&BigNumRef> for &BigNumRef {
    type Output = BigNum;

    fn sub(self, oth: &BigNumRef) -> BigNum {
        let mut r = BigNum::new().unwrap();
        r.checked_sub(self, oth).unwrap();
        r
    }
}

delegate!(Sub, sub);

impl Mul<&BigNumRef> for &BigNumRef {
    type Output = BigNum;

    fn mul(self, oth: &BigNumRef) -> BigNum {
        let mut ctx = BigNumContext::new().unwrap();
        let mut r = BigNum::new().unwrap();
        r.checked_mul(self, oth, &mut ctx).unwrap();
        r
    }
}

delegate!(Mul, mul);

impl<'b> Div<&'b BigNumRef> for &BigNumRef {
    type Output = BigNum;

    fn div(self, oth: &'b BigNumRef) -> BigNum {
        let mut ctx = BigNumContext::new().unwrap();
        let mut r = BigNum::new().unwrap();
        r.checked_div(self, oth, &mut ctx).unwrap();
        r
    }
}

delegate!(Div, div);

impl<'b> Rem<&'b BigNumRef> for &BigNumRef {
    type Output = BigNum;

    fn rem(self, oth: &'b BigNumRef) -> BigNum {
        let mut ctx = BigNumContext::new().unwrap();
        let mut r = BigNum::new().unwrap();
        r.checked_rem(self, oth, &mut ctx).unwrap();
        r
    }
}

delegate!(Rem, rem);

impl Shl<i32> for &BigNumRef {
    type Output = BigNum;

    fn shl(self, n: i32) -> BigNum {
        let mut r = BigNum::new().unwrap();
        r.lshift(self, n).unwrap();
        r
    }
}

impl Shl<i32> for &BigNum {
    type Output = BigNum;

    fn shl(self, n: i32) -> BigNum {
        self.deref().shl(n)
    }
}

impl Shr<i32> for &BigNumRef {
    type Output = BigNum;

    fn shr(self, n: i32) -> BigNum {
        let mut r = BigNum::new().unwrap();
        r.rshift(self, n).unwrap();
        r
    }
}

impl Shr<i32> for &BigNum {
    type Output = BigNum;

    fn shr(self, n: i32) -> BigNum {
        self.deref().shr(n)
    }
}

impl Neg for &BigNumRef {
    type Output = BigNum;

    fn neg(self) -> BigNum {
        self.to_owned().unwrap().neg()
    }
}

impl Neg for &BigNum {
    type Output = BigNum;

    fn neg(self) -> BigNum {
        self.deref().neg()
    }
}

impl Neg for BigNum {
    type Output = BigNum;

    fn neg(mut self) -> BigNum {
        let negative = self.is_negative();
        self.set_negative(!negative);
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::bn::BigNum;

    #[test]
    fn test_to_from_slice() {
        let v0 = BigNum::from_u32(10_203_004).unwrap();
        let vec = v0.to_vec();
        let v1 = BigNum::from_slice(&vec).unwrap();

        assert_eq!(v0, v1);
    }

    #[test]
    fn test_negation() {
        let a = BigNum::from_u32(909_829_283).unwrap();

        assert!(!a.is_negative());
        assert!((-a).is_negative());
    }

    #[test]
    fn test_shift() {
        let a = BigNum::from_u32(909_829_283).unwrap();

        assert_eq!(a, &(&a << 1) >> 1);
    }
}
