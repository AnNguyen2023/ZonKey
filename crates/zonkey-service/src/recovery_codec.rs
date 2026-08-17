//! Durable recovery-state codec (M3D-31 / ADR 0035).
//!
//! Platform-neutral, dependency-free encode/decode for the bounded,
//! versioned recovery state file. The format stores only recovery metadata:
//! document URI, UTF-16 range, salted SHA-256 hashes of the expected and
//! replacement tokens, the reconciliation verdict, and a generation marker.
//! Plaintext document text is never encoded. Integrity is a CRC32 over the
//! whole file; any malformation, truncation, oversize, unknown version, or
//! checksum mismatch decodes to an error, and callers must fail closed.
//!
//! Layout (all integers little-endian):
//! `magic(8) version(u32) salt(16) count(u32) payload_len(u32)
//! payload(records) crc32(u32)`; the CRC covers every preceding byte.

use crate::transport::RecoveryVerdict;

/// File magic; version 1.
pub const RECOVERY_STATE_MAGIC: [u8; 8] = *b"ZNKYREC1";
/// Current format version.
pub const RECOVERY_STATE_VERSION: u32 = 2;
/// Hard cap on persisted records (ADR 0035 registry capacity).
pub const MAX_RECOVERY_ENTRIES: usize = 128;
/// Hard cap on the whole state file.
pub const MAX_RECOVERY_STATE_BYTES: usize = 256 * 1024;
/// Per-file random salt length.
pub const SALT_BYTES: usize = 16;

/// Hard cap on one persisted URI.
const MAX_URI_BYTES: usize = 1024;

/// Hard cap on one persisted request id.
const MAX_REQUEST_ID_BYTES: usize = 256;

/// The lifecycle kind of one persisted record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistedKind {
    /// A blocked target: awaiting reconciliation, or with a recorded
    /// verdict awaiting acknowledgement.
    Blocked { verdict: Option<RecoveryVerdict> },
    /// A durable preflight intent (M3D-31): written before the carrying
    /// request may proceed, removed on a definitive no-mutation rejection,
    /// and promoted to a block on any uncertain outcome.
    Pending,
}

/// Fail-closed codec errors; callers must treat the file as unreadable and
/// never as empty state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryCodecError {
    /// The file is shorter than the fixed header.
    Truncated,
    /// The magic bytes do not match.
    BadMagic,
    /// The version field is not this format version.
    UnknownVersion,
    /// The recorded count exceeds [`MAX_RECOVERY_ENTRIES`].
    TooManyEntries,
    /// The file exceeds [`MAX_RECOVERY_STATE_BYTES`].
    OversizedFile,
    /// A record violates its field bounds.
    OversizedRecord,
    /// Declared lengths disagree with the actual bytes.
    Malformed,
    /// The CRC32 did not match.
    ChecksumMismatch,
    /// Encoding would violate a bound.
    EncodeOverflow,
}

/// One persisted recovery record: metadata only, never plaintext text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedTarget {
    /// Document URI of the logical target.
    pub uri: String,
    /// UTF-16 range of the readback comparison.
    pub range: (usize, usize),
    /// Salted SHA-256 of the expected (rendered) token.
    pub expected_hash: [u8; 32],
    /// Salted SHA-256 of the intended replacement.
    pub replacement_hash: [u8; 32],
    /// Lifecycle kind: blocked (optionally with verdict) or pending.
    pub kind: PersistedKind,
    /// Service-local plan generation at block time.
    pub generation: u64,
    /// Host document open-instance epoch bound to this target; zero when
    /// the record carries no document-epoch binding (operator-created).
    pub document_epoch: u64,
    /// Request id that created the record (pending preflight identity).
    pub request_id: String,
}

/// Salted SHA-256 of one token: `SHA-256(salt || utf8(token))`.
#[must_use]
pub fn salted_hash(salt: &[u8; SALT_BYTES], text: &str) -> [u8; 32] {
    let mut input = Vec::with_capacity(SALT_BYTES + text.len());
    input.extend_from_slice(salt);
    input.extend_from_slice(text.as_bytes());
    sha256(&input)
}

/// Clean-room SHA-256 (FIPS 180-4); verified against NIST vectors in tests.
// The compression routine is the standard published form; the single-letter
// state names and straight-line length follow FIPS notation.
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut message = data.to_vec();
    let bit_length = u64::try_from(data.len()).expect("length fits u64") * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Clean-room CRC-32 (IEEE 802.3, reflected); verified against the standard
/// check value in tests.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = u32::from(crc & 1 != 0).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, RecoveryCodecError> {
    let end = offset.checked_add(2).ok_or(RecoveryCodecError::Malformed)?;
    let slice = bytes
        .get(*offset..end)
        .ok_or(RecoveryCodecError::Truncated)?;
    *offset = end;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, RecoveryCodecError> {
    let end = offset.checked_add(4).ok_or(RecoveryCodecError::Malformed)?;
    let slice = bytes
        .get(*offset..end)
        .ok_or(RecoveryCodecError::Truncated)?;
    *offset = end;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, RecoveryCodecError> {
    let end = offset.checked_add(8).ok_or(RecoveryCodecError::Malformed)?;
    let slice = bytes
        .get(*offset..end)
        .ok_or(RecoveryCodecError::Truncated)?;
    *offset = end;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn read_fixed<const N: usize>(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<[u8; N], RecoveryCodecError> {
    let end = offset.checked_add(N).ok_or(RecoveryCodecError::Malformed)?;
    let slice = bytes
        .get(*offset..end)
        .ok_or(RecoveryCodecError::Truncated)?;
    *offset = end;
    Ok(slice.try_into().expect("fixed length slice"))
}

/// Encodes the salt and records into the bounded versioned file format.
///
/// # Errors
///
/// Returns [`RecoveryCodecError::EncodeOverflow`] when the record count,
/// URI length, or total size would exceed the hard caps.
pub fn encode(
    salt: &[u8; SALT_BYTES],
    records: &[PersistedTarget],
) -> Result<Vec<u8>, RecoveryCodecError> {
    if records.len() > MAX_RECOVERY_ENTRIES {
        return Err(RecoveryCodecError::EncodeOverflow);
    }
    let mut payload = Vec::new();
    for record in records {
        let uri = record.uri.as_bytes();
        if uri.is_empty() || uri.len() > MAX_URI_BYTES {
            return Err(RecoveryCodecError::EncodeOverflow);
        }
        let request_id = record.request_id.as_bytes();
        if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_BYTES {
            return Err(RecoveryCodecError::EncodeOverflow);
        }
        if record.range.0 > record.range.1 {
            return Err(RecoveryCodecError::EncodeOverflow);
        }
        push_u16(
            &mut payload,
            u16::try_from(uri.len()).map_err(|_| RecoveryCodecError::EncodeOverflow)?,
        );
        payload.extend_from_slice(uri);
        push_u64(&mut payload, record.range.0 as u64);
        push_u64(&mut payload, record.range.1 as u64);
        payload.extend_from_slice(&record.expected_hash);
        payload.extend_from_slice(&record.replacement_hash);
        match record.kind {
            PersistedKind::Blocked { verdict: None } => payload.push(0),
            PersistedKind::Blocked {
                verdict: Some(verdict),
            } => {
                payload.push(1);
                payload.push(match verdict {
                    RecoveryVerdict::AppliedAcknowledged => 0,
                    RecoveryVerdict::NotApplied => 1,
                    RecoveryVerdict::ConflictHumanReview => 2,
                });
            }
            PersistedKind::Pending => payload.push(2),
        }
        push_u64(&mut payload, record.generation);
        push_u64(&mut payload, record.document_epoch);
        push_u16(
            &mut payload,
            u16::try_from(request_id.len()).map_err(|_| RecoveryCodecError::EncodeOverflow)?,
        );
        payload.extend_from_slice(request_id);
    }
    let mut file = Vec::with_capacity(payload.len() + 48);
    file.extend_from_slice(&RECOVERY_STATE_MAGIC);
    push_u32(&mut file, RECOVERY_STATE_VERSION);
    file.extend_from_slice(salt);
    let count = u32::try_from(records.len()).map_err(|_| RecoveryCodecError::EncodeOverflow)?;
    push_u32(&mut file, count);
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| RecoveryCodecError::EncodeOverflow)?;
    push_u32(&mut file, payload_len);
    file.extend_from_slice(&payload);
    if file.len() + 4 > MAX_RECOVERY_STATE_BYTES {
        return Err(RecoveryCodecError::EncodeOverflow);
    }
    let checksum = crc32(&file);
    push_u32(&mut file, checksum);
    Ok(file)
}

/// Decodes and validates the bounded versioned file format, returning the
/// file salt and the persisted records.
///
/// # Errors
///
/// Returns the matching [`RecoveryCodecError`] for every malformation;
/// callers must fail closed and never treat a decode failure as empty
/// state.
pub fn decode(
    bytes: &[u8],
) -> Result<([u8; SALT_BYTES], Vec<PersistedTarget>), RecoveryCodecError> {
    if bytes.len() > MAX_RECOVERY_STATE_BYTES {
        return Err(RecoveryCodecError::OversizedFile);
    }
    let mut offset = 0usize;
    if read_fixed::<8>(bytes, &mut offset)? != RECOVERY_STATE_MAGIC {
        return Err(RecoveryCodecError::BadMagic);
    }
    if read_u32(bytes, &mut offset)? != RECOVERY_STATE_VERSION {
        return Err(RecoveryCodecError::UnknownVersion);
    }
    let salt = read_fixed::<SALT_BYTES>(bytes, &mut offset)?;
    let count = read_u32(bytes, &mut offset)?;
    let count = usize::try_from(count).map_err(|_| RecoveryCodecError::Malformed)?;
    if count > MAX_RECOVERY_ENTRIES {
        return Err(RecoveryCodecError::TooManyEntries);
    }
    let payload_len = read_u32(bytes, &mut offset)?;
    let payload_len = usize::try_from(payload_len).map_err(|_| RecoveryCodecError::Malformed)?;
    let payload_end = offset
        .checked_add(payload_len)
        .ok_or(RecoveryCodecError::Malformed)?;
    if bytes.len()
        != payload_end
            .checked_add(4)
            .ok_or(RecoveryCodecError::Malformed)?
    {
        return Err(RecoveryCodecError::Malformed);
    }
    let mut crc_offset = payload_end;
    let stored_crc = read_u32(bytes, &mut crc_offset)?;
    if crc32(&bytes[..bytes.len() - 4]) != stored_crc {
        return Err(RecoveryCodecError::ChecksumMismatch);
    }
    let payload = &bytes[offset..offset + payload_len];
    let mut cursor = 0usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(decode_record(payload, &mut cursor)?);
    }
    if cursor != payload_len {
        return Err(RecoveryCodecError::Malformed);
    }
    Ok((salt, records))
}

/// Decodes one record from the payload at the cursor.
fn decode_record(
    payload: &[u8],
    cursor: &mut usize,
) -> Result<PersistedTarget, RecoveryCodecError> {
    let uri_len = read_u16(payload, cursor)? as usize;
    if uri_len == 0 || uri_len > MAX_URI_BYTES {
        return Err(RecoveryCodecError::OversizedRecord);
    }
    let uri_end = cursor
        .checked_add(uri_len)
        .ok_or(RecoveryCodecError::Malformed)?;
    let uri = std::str::from_utf8(
        payload
            .get(*cursor..uri_end)
            .ok_or(RecoveryCodecError::Truncated)?,
    )
    .map_err(|_| RecoveryCodecError::Malformed)?
    .to_owned();
    *cursor = uri_end;
    let range_start = read_u64(payload, cursor)?;
    let range_end = read_u64(payload, cursor)?;
    let expected_hash = read_fixed::<32>(payload, cursor)?;
    let replacement_hash = read_fixed::<32>(payload, cursor)?;
    let state = *payload.get(*cursor).ok_or(RecoveryCodecError::Truncated)?;
    *cursor += 1;
    let kind = match state {
        0 => PersistedKind::Blocked { verdict: None },
        1 => {
            let code = *payload.get(*cursor).ok_or(RecoveryCodecError::Truncated)?;
            *cursor += 1;
            PersistedKind::Blocked {
                verdict: Some(match code {
                    0 => RecoveryVerdict::AppliedAcknowledged,
                    1 => RecoveryVerdict::NotApplied,
                    2 => RecoveryVerdict::ConflictHumanReview,
                    _ => return Err(RecoveryCodecError::Malformed),
                }),
            }
        }
        2 => PersistedKind::Pending,
        _ => return Err(RecoveryCodecError::Malformed),
    };
    let generation = read_u64(payload, cursor)?;
    let document_epoch = read_u64(payload, cursor)?;
    let request_id_len = read_u16(payload, cursor)? as usize;
    if request_id_len == 0 || request_id_len > MAX_REQUEST_ID_BYTES {
        return Err(RecoveryCodecError::OversizedRecord);
    }
    let request_id_end = cursor
        .checked_add(request_id_len)
        .ok_or(RecoveryCodecError::Malformed)?;
    let request_id = std::str::from_utf8(
        payload
            .get(*cursor..request_id_end)
            .ok_or(RecoveryCodecError::Truncated)?,
    )
    .map_err(|_| RecoveryCodecError::Malformed)?
    .to_owned();
    *cursor = request_id_end;
    Ok(PersistedTarget {
        uri,
        range: (
            usize::try_from(range_start).map_err(|_| RecoveryCodecError::Malformed)?,
            usize::try_from(range_end).map_err(|_| RecoveryCodecError::Malformed)?,
        ),
        expected_hash,
        replacement_hash,
        kind,
        generation,
        document_epoch,
        request_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(verdict: Option<RecoveryVerdict>) -> PersistedTarget {
        let salt = [7u8; SALT_BYTES];
        PersistedTarget {
            uri: "file:///doc/a.txt".to_owned(),
            range: (0, 6),
            expected_hash: salted_hash(&salt, "resume"),
            replacement_hash: salted_hash(&salt, "restored"),
            kind: PersistedKind::Blocked { verdict },
            generation: 3,
            document_epoch: 7,
            request_id: "req-1".to_owned(),
        }
    }

    fn pending_sample() -> PersistedTarget {
        let salt = [7u8; SALT_BYTES];
        PersistedTarget {
            uri: "file:///doc/b.txt".to_owned(),
            range: (2, 8),
            expected_hash: salted_hash(&salt, "resume"),
            replacement_hash: salted_hash(&salt, "restored"),
            kind: PersistedKind::Pending,
            generation: 4,
            document_epoch: 0,
            request_id: "req-pending-9".to_owned(),
        }
    }

    #[test]
    fn sha256_matches_nist_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(text, "{byte:02x}");
        }
        text
    }

    #[test]
    fn clean_roundtrip_for_all_verdict_states() {
        for verdict in [
            None,
            Some(RecoveryVerdict::AppliedAcknowledged),
            Some(RecoveryVerdict::NotApplied),
            Some(RecoveryVerdict::ConflictHumanReview),
        ] {
            let salt = [9u8; SALT_BYTES];
            let file = encode(&salt, &[sample(verdict)]).expect("encode");
            let (loaded_salt, records) = decode(&file).expect("decode");
            assert_eq!(loaded_salt, salt);
            assert_eq!(records, vec![sample(verdict)]);
        }
    }

    #[test]
    fn pending_records_roundtrip() {
        let salt = [11u8; SALT_BYTES];
        let records = vec![pending_sample(), sample(None)];
        let file = encode(&salt, &records).expect("encode");
        let (loaded_salt, loaded) = decode(&file).expect("decode");
        assert_eq!(loaded_salt, salt);
        assert_eq!(loaded, records);
    }

    #[test]
    fn encoding_is_deterministic_within_salt() {
        let salt = [1u8; SALT_BYTES];
        let first = encode(&salt, &[sample(None)]).expect("encode");
        let second = encode(&salt, &[sample(None)]).expect("encode");
        assert_eq!(first, second);
        let other = encode(&[2u8; SALT_BYTES], &[sample(None)]).expect("encode");
        assert_ne!(first, other);
    }

    #[test]
    fn salted_hash_depends_on_salt_and_text() {
        let a = salted_hash(&[1u8; SALT_BYTES], "resume");
        let b = salted_hash(&[2u8; SALT_BYTES], "resume");
        let c = salted_hash(&[1u8; SALT_BYTES], "restored");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn plaintext_tokens_never_appear_in_encoded_file() {
        let salt = [3u8; SALT_BYTES];
        let file = encode(&salt, &[sample(None)]).expect("encode");
        assert!(!file.windows(6).any(|window| window == b"resume"));
        assert!(!file.windows(8).any(|window| window == b"restored"));
        assert!(
            file.windows(17)
                .any(|window| window == b"file:///doc/a.txt")
        );
    }

    #[test]
    fn truncated_file_fails_closed() {
        let salt = [4u8; SALT_BYTES];
        let file = encode(&salt, &[sample(None)]).expect("encode");
        for cut in [1usize, 10, file.len() - 1] {
            assert!(
                matches!(
                    decode(&file[..cut]),
                    Err(RecoveryCodecError::Truncated | RecoveryCodecError::Malformed)
                ),
                "cut={cut}"
            );
        }
    }

    #[test]
    fn bad_magic_version_and_checksum_fail_closed() {
        let salt = [5u8; SALT_BYTES];
        let mut file = encode(&salt, &[sample(None)]).expect("encode");
        file[0] = b'X';
        assert_eq!(decode(&file), Err(RecoveryCodecError::BadMagic));
        let mut file = encode(&salt, &[sample(None)]).expect("encode");
        file[8] = 0xFF;
        assert_eq!(decode(&file), Err(RecoveryCodecError::UnknownVersion));
        let mut file = encode(&salt, &[sample(None)]).expect("encode");
        let last = file.len() - 1;
        file[last] ^= 0x01;
        assert_eq!(decode(&file), Err(RecoveryCodecError::ChecksumMismatch));
        let mut file = encode(&salt, &[sample(None)]).expect("encode");
        file[20] ^= 0x01;
        assert_eq!(decode(&file), Err(RecoveryCodecError::ChecksumMismatch));
    }

    #[test]
    fn oversized_count_and_file_fail_closed() {
        let salt = [6u8; SALT_BYTES];
        let mut file = encode(&salt, &[]).expect("encode");
        let count_offset = 8 + 4 + SALT_BYTES;
        file[count_offset] = 0xFF;
        file[count_offset + 1] = 0xFF;
        file[count_offset + 2] = 0xFF;
        file[count_offset + 3] = 0x7F;
        let crc = crc32(&file);
        file.extend_from_slice(&crc.to_le_bytes());
        assert_eq!(decode(&file), Err(RecoveryCodecError::TooManyEntries));

        let mut big = encode(&salt, &[]).expect("encode");
        big.truncate(big.len() - 4);
        big.extend(std::iter::repeat_n(0u8, MAX_RECOVERY_STATE_BYTES));
        assert_eq!(decode(&big), Err(RecoveryCodecError::OversizedFile));
    }

    #[test]
    fn payload_length_mismatch_fails_closed() {
        let salt = [8u8; SALT_BYTES];
        let mut file = encode(&salt, &[sample(None)]).expect("encode");
        let len_offset = 8 + 4 + SALT_BYTES + 4;
        file[len_offset] ^= 0x10;
        let crc = crc32(&file);
        file.truncate(file.len() - 4);
        file.extend_from_slice(&crc.to_le_bytes());
        assert_eq!(decode(&file), Err(RecoveryCodecError::Malformed));
    }

    #[test]
    fn encode_caps_entries_and_uri() {
        let salt = [2u8; SALT_BYTES];
        let many: Vec<PersistedTarget> = std::iter::repeat_with(|| sample(None))
            .take(MAX_RECOVERY_ENTRIES + 1)
            .collect();
        assert_eq!(
            encode(&salt, &many),
            Err(RecoveryCodecError::EncodeOverflow)
        );
        let mut long = sample(None);
        long.uri = "x".repeat(MAX_URI_BYTES + 1);
        assert_eq!(
            encode(&salt, &[long]),
            Err(RecoveryCodecError::EncodeOverflow)
        );
    }
}
