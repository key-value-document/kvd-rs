//! Strict base64 for the schema-level `bytes` type (spec §5).
//!
//! `bytes` has no token of its own: a `bytes`-typed key holds an ordinary
//! quoted string whose text is base64. This module is that decoding rule.
//!
//! Two deliberate choices, both from the spec:
//!
//! - **Line breaks are layout.** CR and LF are dropped before decoding, so a
//!   value may be wrapped and a `"""` block works with either closer form.
//! - **Spaces and tabs are content.** They are *not* dropped: a space in the
//!   middle of a blob is a bad paste far more often than it is layout.
//!
//! Decoding is otherwise strict: standard alphabet only, padding only at the
//! end, length a multiple of 4, and pad bits zero (RFC 4648 §3.5) so that a
//! decoded value has exactly one representation modulo line breaks.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Why a `bytes` value's text is not valid base64.
///
/// Each variant names the rule that failed, as the error model requires
/// (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BytesError {
    /// A space or tab appears inside the value. Only CR and LF are ignored.
    Whitespace,
    /// The text is not a multiple of 4 characters once line breaks are dropped.
    Length,
    /// A character is outside the standard alphabet (`A-Z`, `a-z`, `0-9`, `+`, `/`).
    Alphabet,
    /// `=` appears somewhere other than as one or two trailing pad characters.
    Padding,
    /// The pad bits of the last encoded character are non-zero (RFC 4648 §3.5),
    /// so the encoding is not canonical.
    PadBits,
}

impl fmt::Display for BytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            BytesError::Whitespace => {
                "space or tab inside a bytes value (only line breaks are ignored)"
            }
            BytesError::Length => "base64 length is not a multiple of 4",
            BytesError::Alphabet => "invalid base64 character",
            BytesError::Padding => "misplaced or excessive `=` padding",
            BytesError::PadBits => "non-zero pad bits (non-canonical base64)",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for BytesError {}

/// The 6-bit value of a base64 character, or `None` if it is not in the
/// standard alphabet.
const fn sextet(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Checks the structural rules and returns the number of data characters
/// (the input length minus trailing padding), or the rule that failed.
///
/// This is the allocation-free core: [`decoded_len`] and [`decode`] both go
/// through it, and schema verification only needs it to answer "how many
/// octets would this be?" (spec §9).
fn scan(text: &str) -> Result<usize, BytesError> {
    let mut total = 0usize; // non-newline characters seen
    let mut data_len = 0usize; // non-newline characters up to the last sextet
    let mut pad = 0usize; // trailing `=` characters
    let mut data_after_pad = false;
    for &c in text.as_bytes() {
        // Layout, not data (spec §5).
        if c == b'\r' || c == b'\n' {
            continue;
        }
        // Content: never ignored.
        if c == b' ' || c == b'\t' {
            return Err(BytesError::Whitespace);
        }
        total += 1;
        if c == b'=' {
            pad += 1;
            continue;
        }
        if sextet(c).is_none() {
            return Err(BytesError::Alphabet);
        }
        if pad > 0 {
            data_after_pad = true;
        }
        data_len = total;
    }
    // `=` belongs only in the tail.
    if data_after_pad || pad > 2 {
        return Err(BytesError::Padding);
    }
    // The whole text, padding included, must be whole quartets.
    if total % 4 != 0 {
        return Err(BytesError::Length);
    }
    Ok(data_len)
}

/// The octet count produced by `data_len` data characters, where the
/// remainder selects the final partial quantum.
fn octets_for(data_len: usize) -> usize {
    data_len / 4 * 3
        + match data_len % 4 {
            0 => 0,
            2 => 1,
            3 => 2,
            _ => unreachable!("scan rejects a 1-char remainder"),
        }
}

/// The number of octets `text` decodes to, or the rule that failed.
///
/// Used by schema verification for `min_len` / `max_len`, which count
/// **decoded octets** (spec §9), never base64 characters.
pub fn decoded_len(text: &str) -> Result<usize, BytesError> {
    let data_len = scan(text)?;
    // Pad bits must be zero for the final partial quantum (RFC 4648 §3.5).
    check_pad_bits(text, data_len)?;
    Ok(octets_for(data_len))
}

/// Rejects a final quantum whose discarded low bits are non-zero.
fn check_pad_bits(text: &str, data_len: usize) -> Result<(), BytesError> {
    let rem = data_len % 4;
    if rem == 0 {
        return Ok(());
    }
    // Last data character, skipping line breaks.
    let mut last = None;
    let mut seen = 0usize;
    for c in text.bytes() {
        if c == b'\r' || c == b'\n' {
            continue;
        }
        seen += 1;
        if seen == data_len {
            last = Some(c);
            break;
        }
    }
    let last = last.unwrap_or(b'A');
    let v = sextet(last).ok_or(BytesError::Alphabet)?;
    // 4 pad bits when one octet remains, 2 when two remain.
    let mask = if rem == 2 { 0b1111 } else { 0b0011 };
    if v & mask != 0 {
        return Err(BytesError::PadBits);
    }
    Ok(())
}

/// Decodes a `bytes` value's text into octets.
///
/// Line breaks are dropped; every other rule in the module docs applies.
/// The empty string decodes to zero octets.
pub fn decode(text: &str) -> Result<Vec<u8>, BytesError> {
    let data_len = scan(text)?;
    check_pad_bits(text, data_len)?;
    let mut out = Vec::with_capacity(octets_for(data_len));
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut seen = 0usize;
    for c in text.bytes() {
        if c == b'\r' || c == b'\n' {
            continue;
        }
        if seen >= data_len {
            break; // padding
        }
        seen += 1;
        let v = sextet(c).ok_or(BytesError::Alphabet)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

/// Encodes octets as canonical base64: standard alphabet, padded, single
/// line, zero pad bits (spec §5).
pub fn encode(octets: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(octets.len().div_ceil(3) * 4);
    for chunk in octets.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}
