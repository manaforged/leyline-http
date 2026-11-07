use std::ops::Range;
use std::ptr::NonNull;

use crate::error::ErrorStack;
use crate::ffi;
use crate::foreign_types::ForeignTypeRef;
use crate::ssl::SslRef;
use crate::{cvt, cvt_p};

/// The record content type of application data.
pub const APPLICATION_DATA: u8 = 23;
/// The record content type of a handshake message.
pub const HANDSHAKE: u8 = 22;
/// The record content type of an alert.
pub const ALERT: u8 = 21;
/// The largest plaintext one record carries.
pub const MAX_PLAINTEXT: usize = 16 * 1024;
/// The length of a record header.
pub const HEADER_LEN: usize = 5;
/// The largest record body a peer may send: plaintext, content type, padding, and tag.
pub const MAX_BODY: usize = MAX_PLAINTEXT + 256;

const TAG_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const MAX_SECRET: usize = 48;
const KEY_UPDATE: u8 = 24;
const NEW_SESSION_TICKET: u8 = 4;

struct Suite {
    aead: *const ffi::EVP_AEAD,
    digest: *const ffi::EVP_MD,
}

impl Suite {
    fn of(ssl: &SslRef) -> Option<Self> {
        // SAFETY: `ssl` is a live SSL whose handshake has finished, so its cipher is set.
        let id = unsafe {
            let cipher = ffi::SSL_get_current_cipher(ssl.as_ptr());
            if cipher.is_null() {
                return None;
            }
            ffi::SSL_CIPHER_get_protocol_id(cipher)
        };
        // SAFETY: the AEAD and digest getters return static tables.
        unsafe {
            match id {
                0x1301 => Some(Self {
                    aead: ffi::EVP_aead_aes_128_gcm(),
                    digest: ffi::EVP_sha256(),
                }),
                0x1302 => Some(Self {
                    aead: ffi::EVP_aead_aes_256_gcm(),
                    digest: ffi::EVP_sha384(),
                }),
                0x1303 => Some(Self {
                    aead: ffi::EVP_aead_chacha20_poly1305(),
                    digest: ffi::EVP_sha256(),
                }),
                _ => None,
            }
        }
    }

    fn expand(&self, secret: &[u8], label: &[u8], len: usize) -> Result<Vec<u8>, ErrorStack> {
        let mut info = Vec::with_capacity(10 + label.len());
        info.extend_from_slice(&u16::try_from(len).unwrap_or(u16::MAX).to_be_bytes());
        info.push(u8::try_from(6 + label.len()).unwrap_or(u8::MAX));
        info.extend_from_slice(b"tls13 ");
        info.extend_from_slice(label);
        info.push(0);
        let mut out = vec![0; len];
        // SAFETY: every pointer covers the length passed beside it.
        cvt(unsafe {
            ffi::HKDF_expand(
                out.as_mut_ptr(),
                len,
                self.digest,
                secret.as_ptr(),
                secret.len(),
                info.as_ptr(),
                info.len(),
            )
        })?;
        Ok(out)
    }
}

struct Direction {
    ctx: NonNull<ffi::EVP_AEAD_CTX>,
    iv: [u8; NONCE_LEN],
    secret: Vec<u8>,
    seq: u64,
}

// SAFETY: the AEAD context is owned by this value and only used through `&mut self`.
unsafe impl Send for Direction {}
// SAFETY: no method reads the context through `&self`.
unsafe impl Sync for Direction {}

impl Direction {
    fn new(suite: &Suite, secret: Vec<u8>, seq: u64) -> Result<Self, ErrorStack> {
        // SAFETY: the AEAD table is static.
        let key_len = unsafe { ffi::EVP_AEAD_key_length(suite.aead) };
        let key = suite.expand(&secret, b"key", key_len)?;
        let iv = suite.expand(&secret, b"iv", NONCE_LEN)?;
        // SAFETY: `key` holds `key_len` bytes.
        let ctx =
            cvt_p(unsafe { ffi::EVP_AEAD_CTX_new(suite.aead, key.as_ptr(), key.len(), TAG_LEN) })?;
        let mut nonce = [0; NONCE_LEN];
        nonce.copy_from_slice(&iv);
        Ok(Self {
            ctx: NonNull::new(ctx).ok_or_else(ErrorStack::get)?,
            iv: nonce,
            secret,
            seq,
        })
    }

    fn next(&self, suite: &Suite) -> Result<Self, ErrorStack> {
        let secret = suite.expand(&self.secret, b"traffic upd", self.secret.len())?;
        Self::new(suite, secret, 0)
    }

    fn nonce(&mut self) -> [u8; NONCE_LEN] {
        let mut nonce = self.iv;
        for (byte, seq) in nonce[NONCE_LEN - 8..]
            .iter_mut()
            .zip(self.seq.to_be_bytes())
        {
            *byte ^= seq;
        }
        self.seq += 1;
        nonce
    }
}

impl Drop for Direction {
    fn drop(&mut self) {
        // SAFETY: the context came from EVP_AEAD_CTX_new and is freed once.
        unsafe { ffi::EVP_AEAD_CTX_free(self.ctx.as_ptr()) }
    }
}

/// What a post-handshake message asks of the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostHandshake {
    /// Nothing more to do.
    Done,
    /// The peer asked for a KeyUpdate in return.
    UpdateRequested,
}

/// The TLS 1.3 application record protection of a connection, taken over after its handshake.
pub struct Records {
    suite: Suite,
    read: Direction,
    write: Direction,
    key_updates: u8,
}

const MAX_KEY_UPDATES: u8 = 32;

// SAFETY: `Suite` holds pointers to static tables, and `Direction` is `Send`.
unsafe impl Send for Records {}
// SAFETY: every method that touches FFI state takes `&mut self`.
unsafe impl Sync for Records {}

impl std::fmt::Debug for Records {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Records")
            .field("read_seq", &self.read.seq)
            .field("write_seq", &self.write.seq)
            .finish_non_exhaustive()
    }
}

impl Records {
    /// Derives the record keys of a TLS 1.3 connection whose handshake finished and whose SSL holds no unread data.
    #[must_use]
    pub fn new(ssl: &SslRef) -> Option<Self> {
        let (suite, read, write) = Self::prepare(ssl)?;
        Some(Self {
            suite,
            read,
            write,
            key_updates: 0,
        })
    }

    fn prepare(ssl: &SslRef) -> Option<(Suite, Direction, Direction)> {
        let raw = ssl.as_ptr();
        // SAFETY: `raw` is a live SSL.
        let ready = unsafe {
            ffi::SSL_version(raw) == ffi::TLS1_3_VERSION
                && ffi::SSL_in_init(raw) == 0
                && ffi::SSL_in_early_data(raw) == 0
                && ffi::SSL_has_pending(raw) == 0
        };
        if !ready {
            return None;
        }
        let suite = Suite::of(ssl)?;
        let mut read = [0; MAX_SECRET];
        let mut write = [0; MAX_SECRET];
        let mut len = 0;
        // SAFETY: both buffers hold `MAX_SECRET` bytes.
        let copied = unsafe {
            ffi::SSL_copy_traffic_secrets(
                raw,
                read.as_mut_ptr(),
                write.as_mut_ptr(),
                MAX_SECRET,
                &mut len,
            )
        };
        if copied != 1 {
            return None;
        }
        // SAFETY: `raw` is a live SSL.
        let (read_seq, write_seq) = unsafe {
            (
                ffi::SSL_get_read_sequence(raw),
                ffi::SSL_get_write_sequence(raw),
            )
        };
        let read = Direction::new(&suite, read[..len].to_vec(), read_seq).ok()?;
        let write = Direction::new(&suite, write[..len].to_vec(), write_seq).ok()?;
        Some((suite, read, write))
    }

    /// Decrypts one whole record in place and returns its content type and the plaintext range within `record`.
    pub fn open(&mut self, record: &mut [u8]) -> Result<(u8, Range<usize>), ErrorStack> {
        let (header, body) = record.split_at_mut(HEADER_LEN);
        let nonce = self.read.nonce();
        let mut len = 0;
        // SAFETY: the context is live; input and output are the same buffer, which BoringSSL allows.
        cvt(unsafe {
            ffi::EVP_AEAD_CTX_open(
                self.read.ctx.as_ptr(),
                body.as_mut_ptr(),
                &mut len,
                body.len(),
                nonce.as_ptr(),
                NONCE_LEN,
                body.as_ptr(),
                body.len(),
                header.as_ptr(),
                HEADER_LEN,
            )
        })?;
        let inner = body[..len]
            .iter()
            .rposition(|&b| b != 0)
            .ok_or_else(ErrorStack::get)?;
        if body[inner] == APPLICATION_DATA {
            self.key_updates = 0;
        }
        Ok((body[inner], HEADER_LEN..HEADER_LEN + inner))
    }

    /// Appends one record of `content_type` that carries `plaintext` (at most 16 KiB) to `out`.
    pub fn seal(
        &mut self,
        content_type: u8,
        plaintext: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<(), ErrorStack> {
        let body = plaintext.len() + 1 + TAG_LEN;
        let start = out.len();
        out.reserve(HEADER_LEN + body);
        out.push(APPLICATION_DATA);
        out.extend_from_slice(&[3, 3]);
        out.extend_from_slice(&u16::try_from(body).unwrap_or(u16::MAX).to_be_bytes());
        out.extend_from_slice(plaintext);
        out.push(content_type);
        out.resize(start + HEADER_LEN + body, 0);
        let (header, sealed) = out[start..].split_at_mut(HEADER_LEN);
        let nonce = self.write.nonce();
        let mut len = 0;
        // SAFETY: the context is live; input and output are the same buffer, which BoringSSL allows.
        cvt(unsafe {
            ffi::EVP_AEAD_CTX_seal(
                self.write.ctx.as_ptr(),
                sealed.as_mut_ptr(),
                &mut len,
                sealed.len(),
                nonce.as_ptr(),
                NONCE_LEN,
                sealed.as_ptr(),
                plaintext.len() + 1,
                header.as_ptr(),
                HEADER_LEN,
            )
        })
    }

    /// Applies one whole post-handshake message: a session ticket goes to the SSL, and a KeyUpdate replaces the read key.
    pub fn post_handshake(
        &mut self,
        ssl: &SslRef,
        message: &[u8],
    ) -> Result<PostHandshake, ErrorStack> {
        match message.first().copied() {
            Some(NEW_SESSION_TICKET) => {
                // SAFETY: `ssl` is live and `message` covers its length.
                cvt(unsafe {
                    ffi::SSL_process_post_handshake(ssl.as_ptr(), message.as_ptr(), message.len())
                })?;
                Ok(PostHandshake::Done)
            }
            Some(KEY_UPDATE)
                if message.len() == 5
                    && message[1..4] == [0, 0, 1]
                    && message[4] <= 1
                    && self.key_updates < MAX_KEY_UPDATES =>
            {
                self.key_updates += 1;
                self.read = self.read.next(&self.suite)?;
                Ok(if message[4] == 1 {
                    PostHandshake::UpdateRequested
                } else {
                    PostHandshake::Done
                })
            }
            _ => Err(ErrorStack::get()),
        }
    }

    /// Appends a KeyUpdate record to `out` and moves the write side to the next key.
    pub fn update_write(&mut self, out: &mut Vec<u8>) -> Result<(), ErrorStack> {
        self.seal(HANDSHAKE, &[KEY_UPDATE, 0, 0, 1, 0], out)?;
        self.write = self.write.next(&self.suite)?;
        Ok(())
    }
}
