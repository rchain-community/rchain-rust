//! String/byte hex-conversion syntax (port of `StringSyntax.scala`, `ByteArraySyntax.scala`, and
//! the portable half of `ByteStringSyntax.scala`).
//!
//! `ByteStringSyntax.toDirectByteBuffer` (Java NIO), `toByteVector` (scodec), and
//! `toBlake2b256Hash` (rspace) are deferred.
//!
//! **This module is ported surface, and none of it is called.** Measured 2026-09-24: *no* method of
//! either trait here has a caller anywhere in the workspace — the in-module tests below are the only
//! uses. It is kept because the Scala has these traits and a reader comparing the two trees should
//! find them, not because anything reaches them; deleting the checked half would be a fidelity
//! decision with no defect behind it, so it stays and says this instead.
//!
//! **The lax decoders are not ported** (2026-09-24, U1/C53's tail): the Scala's
//! `unsafeDecodeHex`/`unsafeHexToByteString` — `base16::unsafe_decode`, which silently drops
//! non-hex characters — had no production caller in the workspace and were the footguns AUDIT §1
//! named, so they were *deleted* rather than carried as dead surface: a decoder whose name says
//! "unsafe" and whose body skips validation is the one a future caller reaches for when it wants
//! "just get bytes out of this", which is exactly what `spec/TYPE-SYSTEM.md` §1.6 forbids. The
//! checked forms (`decode_hex`, `hex_to_byte_string`) stay as the ported surface.

use rchain_shared::base16;

/// Extensions on `str` (port of `StringSyntax`).
pub trait StringSyntax {
    /// Decode hex, or `None` on non-hex input (port of `decodeHex`).
    fn decode_hex(&self) -> Option<Vec<u8>>;

    /// Decode hex to bytes, or `None` (port of `hexToByteString`).
    fn hex_to_byte_string(&self) -> Option<Vec<u8>>;

    /// Whether the string is pure ASCII (port of `onlyAscii`).
    fn only_ascii(&self) -> bool;
}

impl StringSyntax for str {
    fn decode_hex(&self) -> Option<Vec<u8>> {
        base16::decode(self)
    }

    fn hex_to_byte_string(&self) -> Option<Vec<u8>> {
        base16::decode(self)
    }

    fn only_ascii(&self) -> bool {
        self.is_ascii()
    }
}

/// Extensions on byte slices (port of `ByteArraySyntax` and the portable `ByteStringSyntax`).
pub trait ByteArraySyntax {
    /// Copy to an owned byte vector (port of `toByteString`).
    fn to_byte_string(&self) -> Vec<u8>;

    /// Encode as lowercase hex (port of `toHexString`).
    fn to_hex_string(&self) -> String;
}

impl ByteArraySyntax for [u8] {
    fn to_byte_string(&self) -> Vec<u8> {
        self.to_vec()
    }

    fn to_hex_string(&self) -> String {
        base16::encode(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_hex_decode() {
        assert_eq!("0f".decode_hex(), Some(vec![0x0f]));
        assert_eq!("zz".decode_hex(), None);
        assert_eq!("f".decode_hex(), None);
    }

    #[test]
    fn string_hex_to_byte_string() {
        assert_eq!("0f".hex_to_byte_string(), Some(vec![0x0f]));
        assert_eq!("zz".hex_to_byte_string(), None);
    }

    #[test]
    fn only_ascii_detects_non_ascii() {
        assert!("abc".only_ascii());
        assert!(!"héllo".only_ascii());
    }

    #[test]
    fn byte_array_hex_round_trip() {
        let bytes = vec![0x12, 0x34, 0xde, 0xf0];
        assert_eq!(bytes.to_hex_string(), "1234def0");
        assert_eq!(bytes.to_byte_string(), bytes);
    }
}
