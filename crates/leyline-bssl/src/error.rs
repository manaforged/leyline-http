use libc::{c_char, c_int, c_uint};
use openssl_macros::corresponds;
use std::borrow::Cow;
use std::error;
use std::ffi::CStr;
use std::ffi::CString;
use std::fmt;
use std::io;
use std::ptr;
use std::str;

use crate::ffi;

pub use crate::ffi::ErrLib;

#[derive(Debug, Clone)]
pub struct ErrorStack(Vec<Error>);

impl ErrorStack {
    #[corresponds(ERR_get_error_line_data)]
    #[must_use = "Use ErrorStack::clear() to drop the error stack"]
    pub fn get() -> ErrorStack {
        let mut vec = vec![];
        while let Some(err) = Error::get() {
            vec.push(err);
        }
        ErrorStack(vec)
    }

    #[corresponds(ERR_put_error)]
    pub fn put(&self) {
        for error in self.errors() {
            error.put();
        }
    }

    #[cold]
    pub(crate) fn internal_error(err: impl error::Error) -> Self {
        Self(vec![Error::new_internal(Data::String(err.to_string()))])
    }

    #[cold]
    pub(crate) fn internal_error_str(message: &'static str) -> Self {
        Self(vec![Error::new_internal(Data::Static(message))])
    }

    #[corresponds(ERR_clear_error)]
    pub(crate) fn clear() {
        unsafe {
            ffi::ERR_clear_error();
        }
    }
}

impl ErrorStack {
    #[must_use]
    pub fn errors(&self) -> &[Error] {
        &self.0
    }
}

impl fmt::Display for ErrorStack {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        if self.0.is_empty() {
            return fmt.write_str("unknown BoringSSL error");
        }

        let mut first = true;
        for err in &self.0 {
            if !first {
                fmt.write_str(" ")?;
            }
            first = false;
            write!(
                fmt,
                "[{}]",
                err.reason()
                    .or_else(|| err.library())
                    .unwrap_or("unknown reason")
            )?;
        }
        Ok(())
    }
}

impl error::Error for ErrorStack {}

impl From<ErrorStack> for io::Error {
    fn from(e: ErrorStack) -> io::Error {
        io::Error::other(e)
    }
}

impl From<ErrorStack> for fmt::Error {
    fn from(_: ErrorStack) -> fmt::Error {
        fmt::Error
    }
}

#[derive(Clone)]
pub struct Error {
    code: c_uint,
    file: *const c_char,
    line: c_uint,
    data: Data,
}

#[derive(Clone)]
enum Data {
    None,
    CString(CString),
    String(String),
    Static(&'static str),
}

unsafe impl Sync for Error {}
unsafe impl Send for Error {}

static BORING_INTERNAL: &CStr = c"boring-rust";

impl Error {
    #[must_use = "Use ErrorStack::clear() to drop the error stack"]
    #[corresponds(ERR_get_error_line_data)]
    pub fn get() -> Option<Error> {
        unsafe {
            ffi::init();

            let mut file = ptr::null();
            let mut line = 0;
            let mut data = ptr::null();
            let mut flags = 0;
            match ffi::ERR_get_error_line_data(&mut file, &mut line, &mut data, &mut flags) {
                0 => None,
                code => {
                    let data = if flags & ffi::ERR_FLAG_STRING != 0 {
                        Data::CString(CStr::from_ptr(data.cast()).to_owned())
                    } else {
                        Data::None
                    };
                    Some(Error {
                        code,
                        file,
                        line: line as c_uint,
                        data,
                    })
                }
            }
        }
    }

    #[corresponds(ERR_put_error)]
    pub fn put(&self) {
        unsafe {
            ffi::ERR_put_error(
                ffi::ERR_GET_LIB(self.code),
                ffi::ERR_GET_FUNC(self.code),
                ffi::ERR_GET_REASON(self.code),
                self.file,
                self.line,
            );
            if let Some(cstr) = self.data_cstr() {
                ffi::ERR_add_error_data(1, cstr.as_ptr().cast_mut());
            }
        }
    }

    #[inline]
    #[must_use]
    #[track_caller]
    pub fn library_reason(&self, library_code: ErrLib) -> Option<c_int> {
        debug_assert!(library_code.0 < ffi::ERR_NUM_LIBS.0);
        (self.library_code() == library_code.0 as c_int).then_some(self.reason_code())
    }

    #[must_use]
    #[deprecated(note = "use library_reason() to compare error codes")]
    pub fn code(&self) -> c_uint {
        self.code
    }

    #[must_use]
    pub fn library(&self) -> Option<&'static str> {
        if self.is_internal() {
            return None;
        }
        unsafe {
            let cstr = ffi::ERR_lib_error_string(self.code);
            if cstr.is_null() {
                return None;
            }
            CStr::from_ptr(cstr.cast())
                .to_str()
                .ok()
                .filter(|&msg| msg != "unknown library")
        }
    }

    #[must_use]
    pub fn library_code(&self) -> c_int {
        ffi::ERR_GET_LIB(self.code)
    }

    pub fn function(&self) -> Option<&'static str> {
        None
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        if self.is_internal() {
            return self.data();
        }
        unsafe {
            let cstr = ffi::ERR_reason_error_string(self.code);
            if cstr.is_null() {
                return None;
            }
            CStr::from_ptr(cstr.cast()).to_str().ok()
        }
    }

    #[must_use]
    pub fn reason_code(&self) -> c_int {
        ffi::ERR_GET_REASON(self.code)
    }

    #[must_use]
    pub fn file(&self) -> &'static str {
        unsafe {
            if self.file.is_null() {
                return "";
            }
            CStr::from_ptr(self.file.cast())
                .to_str()
                .unwrap_or_default()
        }
    }

    #[allow(clippy::unnecessary_cast)]
    #[must_use]
    pub fn line(&self) -> u32 {
        self.line as u32
    }

    #[must_use]
    pub fn data(&self) -> Option<&str> {
        match &self.data {
            Data::None => None,
            Data::CString(cstring) => cstring.to_str().ok(),
            Data::String(s) => Some(s),
            Data::Static(s) => Some(s),
        }
    }

    #[must_use]
    fn data_cstr(&self) -> Option<Cow<'_, CStr>> {
        let s = match &self.data {
            Data::None => return None,
            Data::CString(cstr) => return Some(Cow::Borrowed(cstr)),
            Data::String(s) => s.as_str(),
            Data::Static(s) => s,
        };
        CString::new(s).ok().map(Cow::Owned)
    }

    fn new_internal(msg: Data) -> Self {
        Self {
            code: ffi::ERR_PACK(ffi::ERR_LIB_NONE.0 as _, 0, 0) as _,
            file: BORING_INTERNAL.as_ptr(),
            line: 0,
            data: msg,
        }
    }

    fn is_internal(&self) -> bool {
        std::ptr::eq(self.file, BORING_INTERNAL.as_ptr())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let mut builder = fmt.debug_struct("Error");
        builder.field("code", &self.code);
        if !self.is_internal() {
            if let Some(library) = self.library() {
                builder.field("library", &library);
            }
            builder.field("library_code", &self.library_code());
            if let Some(reason) = self.reason() {
                builder.field("reason", &reason);
            }
            builder.field("reason_code", &self.reason_code());
            builder.field("file", &self.file());
            builder.field("line", &self.line());
        }
        if let Some(data) = self.data() {
            builder.field("data", &data);
        }
        builder.finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(
            fmt,
            "{}\n\nCode: {:08X}\nLoc: {}:{}",
            self.reason().unwrap_or("unknown TLS error"),
            &self.code,
            self.file(),
            self.line()
        )
    }
}

impl error::Error for Error {}

#[test]
fn internal_err() {
    let e = ErrorStack::internal_error(io::Error::other("hello, boring"));
    assert_eq!(1, e.errors().len());
    assert!(e.to_string().contains("hello, boring"), "{e} {e:?}");

    e.put();
    let e = ErrorStack::get();
    assert!(e.to_string().contains("hello, boring"), "{e} {e:?}");
}
