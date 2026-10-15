//! Encrypted container for Money Tracker backups.
//!
//! Knows nothing about what it carries: it turns a byte stream into an
//! authenticated, encrypted file and back, in fixed-size chunks so memory use
//! does not depend on the size of the backup.
//!
//! ```text
//! header (57 bytes, authenticated as AAD of every chunk)
//!   "MTBK" | format u8 | kdf u8 | m_kib u32 | t u32 | p u32 | salt[16] | nonce_prefix[19] | chunk u32
//! frames, each:  len u32 (big endian, ciphertext length) | ciphertext
//! ```
//!
//! The key is Argon2id(passphrase, salt, params). Chunks use XChaCha20-Poly1305
//! in the STREAM (BE32) construction, so reordering, dropping, duplicating or
//! truncating chunks is detected, and the last chunk is marked as last.
//! There is **no recovery** for a lost passphrase.

use std::{
    fmt,
    io::{self, Read, Write},
};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305,
    aead::{
        Payload,
        stream::{DecryptorBE32, EncryptorBE32},
    },
};
use zeroize::Zeroizing;

pub const MAGIC: [u8; 4] = *b"MTBK";
pub const FORMAT_VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_PREFIX_LEN: usize = 19;
const HEADER_LEN: usize = 4 + 1 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_PREFIX_LEN + 4;
const TAG_LEN: usize = 16;

/// Plaintext bytes per chunk.
pub const CHUNK_SIZE: usize = 64 * 1024;
const MAX_CHUNK_SIZE: u32 = 1024 * 1024;

/// Bounds accepted when *reading*, so a crafted file cannot demand absurd work.
const MIN_M_KIB: u32 = 64;
const MAX_M_KIB: u32 = 256 * 1024;
const MAX_T: u32 = 10;
const MAX_P: u32 = 4;

/// Passphrase rules (counted in characters, not bytes).
pub const PASSPHRASE_MIN_CHARS: usize = 12;
pub const PASSPHRASE_MAX_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupError {
    /// Not a Money Tracker backup (wrong magic or too short for a header).
    NotABackup,
    /// Written by a newer format version than this build reads.
    UnsupportedVersion(u8),
    /// Header parameters are out of the accepted range.
    BadParameters,
    /// Wrong passphrase, or the file was altered. The two cannot be told apart.
    WrongPassphraseOrCorrupt,
    /// The file ends before its last chunk.
    Truncated,
    /// Data after the last chunk.
    TrailingData,
    Io,
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotABackup => "not a Money Tracker backup",
            Self::UnsupportedVersion(_) => "backup format version is not supported",
            Self::BadParameters => "backup parameters are out of range",
            Self::WrongPassphraseOrCorrupt => "wrong passphrase or damaged backup",
            Self::Truncated => "backup is incomplete",
            Self::TrailingData => "unexpected data after the end of the backup",
            Self::Io => "could not read or write the backup",
        })
    }
}

impl std::error::Error for BackupError {}

impl BackupError {
    fn into_io(self) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, self)
    }

    /// Recovers the error from an `io::Error` produced by the reader/writer.
    pub fn from_io(err: &io::Error) -> Self {
        err.get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
            .copied()
            .unwrap_or(Self::Io)
    }
}

impl From<io::Error> for BackupError {
    fn from(err: io::Error) -> Self {
        Self::from_io(&err)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassphraseProblem {
    TooShort,
    TooLong,
    /// One character repeated, which is trivially guessable.
    Repetitive,
}

impl fmt::Display for PassphraseProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooShort => "passphrase must be at least 12 characters",
            Self::TooLong => "passphrase must be at most 128 characters",
            Self::Repetitive => "passphrase is too easy to guess",
        })
    }
}

impl std::error::Error for PassphraseProblem {}

/// Checks a passphrase chosen for a new backup. Never echoes it.
pub fn validate_passphrase(passphrase: &str) -> Result<(), PassphraseProblem> {
    let chars = passphrase.chars().count();
    if chars < PASSPHRASE_MIN_CHARS {
        return Err(PassphraseProblem::TooShort);
    }
    if chars > PASSPHRASE_MAX_CHARS {
        return Err(PassphraseProblem::TooLong);
    }
    let mut it = passphrase.chars();
    if let Some(first) = it.next()
        && it.all(|c| c == first)
    {
        return Err(PassphraseProblem::Repetitive);
    }
    Ok(())
}

/// Argon2id cost recorded in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    /// Used for new backups: 64 MiB, 3 passes, 1 lane. A backup file can be
    /// attacked offline for as long as it exists, so this is heavier than the
    /// PIN hash; it runs once per export or restore.
    pub const DEFAULT: Self = Self {
        m_kib: 64 * 1024,
        t: 3,
        p: 1,
    };

    /// Cheap parameters for tests only.
    pub const fn for_tests() -> Self {
        Self {
            m_kib: MIN_M_KIB,
            t: 1,
            p: 1,
        }
    }

    fn in_range(self) -> bool {
        (MIN_M_KIB..=MAX_M_KIB).contains(&self.m_kib)
            && (1..=MAX_T).contains(&self.t)
            && (1..=MAX_P).contains(&self.p)
    }
}

type Key = Zeroizing<[u8; 32]>;

fn derive_key(passphrase: &str, salt: &[u8], params: KdfParams) -> Result<Key, BackupError> {
    let argon = Params::new(params.m_kib, params.t, params.p, Some(32))
        .map_err(|_| BackupError::BadParameters)?;
    let mut key: Key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, argon)
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut_slice())
        .map_err(|_| BackupError::BadParameters)?;
    Ok(key)
}

/// Encrypts everything written to it. Call [`BackupWriter::finish`] to write
/// the final chunk; dropping without finishing leaves a file that fails to open.
pub struct BackupWriter<W: Write> {
    inner: W,
    encryptor: Option<EncryptorBE32<XChaCha20Poly1305>>,
    header: [u8; HEADER_LEN],
    buffer: Vec<u8>,
}

impl<W: Write> BackupWriter<W> {
    pub fn new(inner: W, passphrase: &str, params: KdfParams) -> Result<Self, BackupError> {
        Self::with_chunk(inner, passphrase, params, CHUNK_SIZE as u32)
    }

    fn with_chunk(
        mut inner: W,
        passphrase: &str,
        params: KdfParams,
        chunk: u32,
    ) -> Result<Self, BackupError> {
        if !params.in_range() || chunk == 0 || chunk > MAX_CHUNK_SIZE {
            return Err(BackupError::BadParameters);
        }
        let mut salt = [0u8; SALT_LEN];
        let mut prefix = [0u8; NONCE_PREFIX_LEN];
        getrandom::fill(&mut salt).map_err(|_| BackupError::Io)?;
        getrandom::fill(&mut prefix).map_err(|_| BackupError::Io)?;

        let mut header = [0u8; HEADER_LEN];
        let mut at = 0;
        let mut put = |bytes: &[u8]| {
            header[at..at + bytes.len()].copy_from_slice(bytes);
            at += bytes.len();
        };
        put(&MAGIC);
        put(&[FORMAT_VERSION, KDF_ARGON2ID]);
        put(&params.m_kib.to_be_bytes());
        put(&params.t.to_be_bytes());
        put(&params.p.to_be_bytes());
        put(&salt);
        put(&prefix);
        put(&chunk.to_be_bytes());

        let key = derive_key(passphrase, &salt, params)?;
        let cipher = XChaCha20Poly1305::new(key.as_slice().into());
        let encryptor = EncryptorBE32::from_aead(cipher, (&prefix).into());
        inner.write_all(&header).map_err(|_| BackupError::Io)?;
        Ok(Self {
            inner,
            encryptor: Some(encryptor),
            header,
            buffer: Vec::with_capacity(chunk as usize + 1),
        })
    }

    fn chunk_len(&self) -> usize {
        u32::from_be_bytes([
            self.header[HEADER_LEN - 4],
            self.header[HEADER_LEN - 3],
            self.header[HEADER_LEN - 2],
            self.header[HEADER_LEN - 1],
        ]) as usize
    }

    fn emit(&mut self, frame: &[u8]) -> io::Result<()> {
        let len = u32::try_from(frame.len()).map_err(|_| BackupError::Io.into_io())?;
        self.inner.write_all(&len.to_be_bytes())?;
        self.inner.write_all(frame)
    }

    /// Writes the last chunk and returns the underlying writer.
    pub fn finish(mut self) -> io::Result<W> {
        let encryptor = self
            .encryptor
            .take()
            .ok_or_else(|| BackupError::Io.into_io())?;
        let payload = Payload {
            msg: &self.buffer,
            aad: &self.header,
        };
        let frame = encryptor
            .encrypt_last(payload)
            .map_err(|_| BackupError::Io.into_io())?;
        self.buffer.clear();
        self.emit(&frame)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for BackupWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(data);
        let chunk = self.chunk_len();
        // Only flush a chunk once more data exists, so `finish` always has a
        // (possibly empty) final chunk to mark as last.
        while self.buffer.len() > chunk {
            let rest = self.buffer.split_off(chunk);
            let plain = std::mem::replace(&mut self.buffer, rest);
            let encryptor = self
                .encryptor
                .as_mut()
                .ok_or_else(|| BackupError::Io.into_io())?;
            let frame = encryptor
                .encrypt_next(Payload {
                    msg: &plain,
                    aad: &self.header,
                })
                .map_err(|_| BackupError::Io.into_io())?;
            self.emit(&frame)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Decrypts a backup as it is read. Chunks are authenticated before any of
/// their bytes are returned, and the end of the stream is only reported once
/// the final chunk has verified.
pub struct BackupReader<R: Read> {
    inner: R,
    decryptor: Option<DecryptorBE32<XChaCha20Poly1305>>,
    header: [u8; HEADER_LEN],
    max_frame: usize,
    next: Option<Vec<u8>>,
    plain: Vec<u8>,
    pos: usize,
    finished: bool,
}

impl<R: Read> BackupReader<R> {
    /// Reads the header, derives the key and authenticates the first chunk, so
    /// a wrong passphrase is reported here rather than midway through a restore.
    pub fn open(mut inner: R, passphrase: &str) -> Result<Self, BackupError> {
        let mut header = [0u8; HEADER_LEN];
        read_exact_or(&mut inner, &mut header, BackupError::NotABackup)?;
        if header[..4] != MAGIC {
            return Err(BackupError::NotABackup);
        }
        let version = header[4];
        if version != FORMAT_VERSION {
            return Err(BackupError::UnsupportedVersion(version));
        }
        if header[5] != KDF_ARGON2ID {
            return Err(BackupError::BadParameters);
        }
        let be = |at: usize| {
            u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let params = KdfParams {
            m_kib: be(6),
            t: be(10),
            p: be(14),
        };
        let chunk = be(HEADER_LEN - 4);
        if !params.in_range() || chunk == 0 || chunk > MAX_CHUNK_SIZE {
            return Err(BackupError::BadParameters);
        }
        let salt = &header[18..18 + SALT_LEN];
        let prefix: [u8; NONCE_PREFIX_LEN] = header
            [18 + SALT_LEN..18 + SALT_LEN + NONCE_PREFIX_LEN]
            .try_into()
            .map_err(|_| BackupError::NotABackup)?;

        let key = derive_key(passphrase, salt, params)?;
        let cipher = XChaCha20Poly1305::new(key.as_slice().into());
        let decryptor = DecryptorBE32::from_aead(cipher, (&prefix).into());

        let max_frame = chunk as usize + TAG_LEN;
        let first = read_frame(&mut inner, max_frame)?.ok_or(BackupError::Truncated)?;
        let mut reader = Self {
            inner,
            decryptor: Some(decryptor),
            header,
            max_frame,
            next: Some(first),
            plain: Vec::new(),
            pos: 0,
            finished: false,
        };
        reader
            .load_next()
            .map_err(|err| BackupError::from_io(&err))?;
        Ok(reader)
    }

    /// Decrypts the next frame into `plain`. Returns false at the end.
    fn load_next(&mut self) -> io::Result<bool> {
        let Some(current) = self.next.take() else {
            return Ok(false);
        };
        let following =
            read_frame(&mut self.inner, self.max_frame).map_err(BackupError::into_io)?;
        let payload = Payload {
            msg: &current,
            aad: &self.header,
        };
        let plain = match following {
            Some(next) => {
                let dec = self
                    .decryptor
                    .as_mut()
                    .ok_or_else(|| BackupError::WrongPassphraseOrCorrupt.into_io())?;
                let plain = dec
                    .decrypt_next(payload)
                    .map_err(|_| BackupError::WrongPassphraseOrCorrupt.into_io())?;
                self.next = Some(next);
                plain
            }
            None => {
                let dec = self
                    .decryptor
                    .take()
                    .ok_or_else(|| BackupError::WrongPassphraseOrCorrupt.into_io())?;
                // A file that was cut at a frame boundary fails here: its last
                // frame was not encrypted as the last one.
                let plain = dec
                    .decrypt_last(payload)
                    .map_err(|_| BackupError::WrongPassphraseOrCorrupt.into_io())?;
                self.finished = true;
                plain
            }
        };
        self.plain = plain;
        self.pos = 0;
        Ok(true)
    }
}

impl<R: Read> Read for BackupReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pos < self.plain.len() {
                let n = out.len().min(self.plain.len() - self.pos);
                out[..n].copy_from_slice(&self.plain[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.finished || !self.load_next()? {
                // The last frame is verified: make sure nothing follows it.
                let mut probe = [0u8; 1];
                return match self.inner.read(&mut probe) {
                    Ok(0) => Ok(0),
                    Ok(_) => Err(BackupError::TrailingData.into_io()),
                    Err(err) => Err(err),
                };
            }
        }
    }
}

fn read_exact_or<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    on_short: BackupError,
) -> Result<(), BackupError> {
    reader.read_exact(buf).map_err(|err| {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            on_short
        } else {
            BackupError::Io
        }
    })
}

/// Reads one frame, or `None` at a clean end of file.
fn read_frame<R: Read>(reader: &mut R, max_frame: usize) -> Result<Option<Vec<u8>>, BackupError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < len.len() {
        match reader.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(BackupError::Truncated),
            Ok(n) => got += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(BackupError::Io),
        }
    }
    let size = u32::from_be_bytes(len) as usize;
    if !(TAG_LEN..=max_frame).contains(&size) {
        return Err(BackupError::WrongPassphraseOrCorrupt);
    }
    let mut frame = vec![0u8; size];
    read_exact_or(reader, &mut frame, BackupError::Truncated)?;
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASS: &str = "correct horse battery";

    fn seal(data: &[u8], pass: &str) -> Vec<u8> {
        seal_chunked(data, pass, CHUNK_SIZE as u32)
    }

    fn seal_chunked(data: &[u8], pass: &str, chunk: u32) -> Vec<u8> {
        let mut writer =
            BackupWriter::with_chunk(Vec::new(), pass, KdfParams::for_tests(), chunk).unwrap();
        // Uneven writes exercise the buffering.
        for part in data.chunks(7919) {
            writer.write_all(part).unwrap();
        }
        writer.finish().unwrap()
    }

    fn unseal(file: &[u8], pass: &str) -> Result<Vec<u8>, BackupError> {
        let mut reader = BackupReader::open(file, pass)?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out)?;
        Ok(out)
    }

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn round_trips_empty_small_exact_and_multi_chunk() {
        for len in [
            0usize,
            1,
            100,
            CHUNK_SIZE - 1,
            CHUNK_SIZE,
            CHUNK_SIZE + 1,
            CHUNK_SIZE * 3,
            CHUNK_SIZE * 3 + 17,
        ] {
            let data = sample(len);
            assert_eq!(unseal(&seal(&data, PASS), PASS).unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn small_chunks_round_trip_too() {
        let data = sample(1000);
        assert_eq!(unseal(&seal_chunked(&data, PASS, 16), PASS).unwrap(), data);
    }

    #[test]
    fn output_is_not_plaintext_and_differs_every_time() {
        let data = b"salary 5000000 groceries 25000 ".repeat(200);
        let a = seal(&data, PASS);
        let b = seal(&data, PASS);
        assert_ne!(a, b, "salt and nonce are random");
        assert!(!a.windows(6).any(|w| w == b"salary"));
        assert_eq!(&a[..4], b"MTBK");
    }

    #[test]
    fn wrong_passphrase_is_rejected_at_open() {
        let file = seal(&sample(5000), PASS);
        assert_eq!(
            BackupReader::open(&file[..], "another passphrase").err(),
            Some(BackupError::WrongPassphraseOrCorrupt)
        );
    }

    #[test]
    fn flipping_any_single_byte_is_detected() {
        let data = sample(CHUNK_SIZE + 500);
        let file = seal(&data, PASS);
        // Header, a frame length, early and late ciphertext, and the last tag.
        let positions = [
            0,
            4,
            5,
            6,
            9,
            20,
            30,
            45,
            HEADER_LEN - 1,
            HEADER_LEN,
            HEADER_LEN + 2,
            HEADER_LEN + 1000,
            file.len() / 2,
            file.len() - 1,
        ];
        for &at in &positions {
            let mut bad = file.clone();
            bad[at] ^= 0x01;
            assert!(unseal(&bad, PASS).is_err(), "byte {at} was not detected");
        }
    }

    #[test]
    fn truncation_is_detected_everywhere() {
        let data = sample(CHUNK_SIZE * 2 + 10);
        let file = seal(&data, PASS);
        for cut in [
            0,
            3,
            HEADER_LEN - 1,
            HEADER_LEN,
            HEADER_LEN + 3,
            HEADER_LEN + 4 + CHUNK_SIZE / 2,
            file.len() - 1,
            file.len() - 20,
        ] {
            assert!(unseal(&file[..cut], PASS).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn dropping_the_last_frame_at_a_boundary_is_detected() {
        let data = sample(CHUNK_SIZE * 2 + 10);
        let file = seal(&data, PASS);
        // Find the start of the final frame by walking the frame lengths.
        let mut at = HEADER_LEN;
        let mut last_start = at;
        while at < file.len() {
            last_start = at;
            let len =
                u32::from_be_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]) as usize;
            at += 4 + len;
        }
        assert_eq!(
            unseal(&file[..last_start], PASS),
            Err(BackupError::WrongPassphraseOrCorrupt)
        );
    }

    #[test]
    fn reordered_or_duplicated_frames_are_detected() {
        let file = seal(&sample(CHUNK_SIZE * 3 + 5), PASS);
        let mut frames = Vec::new();
        let mut at = HEADER_LEN;
        while at < file.len() {
            let len =
                u32::from_be_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]) as usize;
            frames.push(file[at..at + 4 + len].to_vec());
            at += 4 + len;
        }
        let mut swapped = file[..HEADER_LEN].to_vec();
        for i in [1usize, 0, 2, 3] {
            swapped.extend_from_slice(&frames[i]);
        }
        assert!(unseal(&swapped, PASS).is_err());
        let mut duplicated = file.clone();
        duplicated.extend_from_slice(&frames[0]);
        assert!(unseal(&duplicated, PASS).is_err());
    }

    #[test]
    fn trailing_garbage_is_detected() {
        let mut file = seal(&sample(100), PASS);
        file.extend_from_slice(b"extra");
        assert!(unseal(&file, PASS).is_err());
    }

    #[test]
    fn unknown_files_and_versions_are_refused() {
        assert_eq!(
            BackupReader::open(&b""[..], PASS).err(),
            Some(BackupError::NotABackup)
        );
        assert_eq!(
            BackupReader::open(&b"MTBK"[..], PASS).err(),
            Some(BackupError::NotABackup)
        );
        assert_eq!(
            BackupReader::open(&[7u8; 200][..], PASS).err(),
            Some(BackupError::NotABackup)
        );
        let mut file = seal(&sample(10), PASS);
        file[4] = 2;
        assert_eq!(
            BackupReader::open(&file[..], PASS).err(),
            Some(BackupError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn crafted_parameters_are_refused_before_any_hashing() {
        let mut file = seal(&sample(10), PASS);
        // m_kib = 4 GiB
        file[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            BackupReader::open(&file[..], PASS).err(),
            Some(BackupError::BadParameters)
        );
        let mut file = seal(&sample(10), PASS);
        file[10..14].copy_from_slice(&1000u32.to_be_bytes());
        assert_eq!(
            BackupReader::open(&file[..], PASS).err(),
            Some(BackupError::BadParameters)
        );
    }

    #[test]
    fn writer_refuses_out_of_range_parameters() {
        let bad = KdfParams {
            m_kib: 1,
            t: 1,
            p: 1,
        };
        assert_eq!(
            BackupWriter::new(Vec::new(), PASS, bad).err(),
            Some(BackupError::BadParameters)
        );
    }

    #[test]
    fn io_error_round_trips_to_a_backup_error() {
        let file = seal(&sample(10), PASS);
        let mut bad = file.clone();
        *bad.last_mut().unwrap() ^= 1;
        let mut reader = BackupReader::open(&bad[..], PASS);
        // The only frame is also the last, so it fails at open.
        assert_eq!(
            reader.as_mut().err().copied(),
            Some(BackupError::WrongPassphraseOrCorrupt)
        );
        let err = BackupError::WrongPassphraseOrCorrupt.into_io();
        assert_eq!(
            BackupError::from_io(&err),
            BackupError::WrongPassphraseOrCorrupt
        );
        assert_eq!(
            BackupError::from_io(&io::Error::other("x")),
            BackupError::Io
        );
    }

    #[test]
    fn passphrase_policy() {
        assert_eq!(
            validate_passphrase("short"),
            Err(PassphraseProblem::TooShort)
        );
        assert_eq!(
            validate_passphrase("elevenchars"),
            Err(PassphraseProblem::TooShort)
        );
        assert_eq!(validate_passphrase("twelve chars"), Ok(()));
        assert_eq!(
            validate_passphrase("aaaaaaaaaaaaaaa"),
            Err(PassphraseProblem::Repetitive)
        );
        assert_eq!(
            validate_passphrase(&"ab".repeat(70)),
            Err(PassphraseProblem::TooLong)
        );
        // Characters, not bytes: 12 multi-byte characters qualify.
        assert_eq!(validate_passphrase("あいうえおかきくけこさし"), Ok(()));
    }
}
