//! Syrup — the binary encoding OCapN carries CapTP messages in.
//!
//! The OCapN draft specifies the wire form as "a subset of [Syrup]"
//! (`draft-specifications/Notation.md`), with the abstract notation borrowed from Preserves. This
//! module implements that concrete grammar: booleans, arbitrary-precision integers, float64,
//! strings, byte arrays, symbols, lists, string-keyed structs, and records.
//!
//! **What this codec deliberately does not invent.** Two parts of the model are left undecided by
//! the draft, and a silent choice there would be a silent incompatibility, so they are *not* fixed
//! here:
//!
//! * OCapN's `Tagged` value crosses as a labelled **record** — the `desc:tagged` form a Rholang
//!   tuple takes (`Rchain.Syrup`'s `taggedRecord`) — so it needs no constructor of its own here.
//!   `Undefined`, `Null`, `Reference` and `Error` (`draft-specifications/Model.md`) have no concrete
//!   Syrup form anywhere in `Notation.md`, and no CapTP message this port speaks carries one, so they
//!   are left unimplemented rather than guessed: a value with no wire form cannot become a silent
//!   incompatibility if it never reaches the wire.
//! * Dictionaries: `Notation.md` gives `{ … }` with unordered key/value pairs; `Model.md` fixes an
//!   OCapN *Struct* to String keys, pairwise non-Equal. This codec implements the Struct — string
//!   keys, duplicates refused — since that is the only keyed container OCapN defines. General
//!   Syrup permits non-string keys; OCapN does not, so neither does this.
//!
//! **Canonical on the way out.** Struct keys are emitted in sorted order, integers carry no
//! leading zero, and a container never carries inter-token whitespace. The decoder is lenient about
//! inter-token whitespace (the grammar ignores SP/HT/CR/LF) but strict about everything else: a
//! trailing byte, a length that overruns the buffer, non-UTF-8 where a string is required, a
//! duplicate struct key, or nesting past [`MAX_DEPTH`] are all *refusals*, never panics and never
//! partial results. Recursive descent over attacker-supplied bytes is a stack-depth DoS unless it
//! is bounded, so it is (the same guard class as `rchain-models`'s normalizer).

use std::collections::BTreeMap;
use std::fmt;

use num_bigint::{BigInt, Sign};

/// The deepest container nesting `from_bytes` will decode before refusing with
/// [`SyrupError::DepthExceeded`]. Far above any CapTP message's nesting, far below any stack limit.
pub const MAX_DEPTH: usize = 256;

/// One Syrup value — the OCapN wire model's atoms and containers.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `f` or `t`.
    Bool(bool),
    /// An arbitrary-precision signed integer, digits followed by `+` or `-`. Zero is `0+`.
    Int(BigInt),
    /// An IEEE 754 binary64: `D` then the 8 network-order bytes.
    Float64(f64),
    /// A Unicode string: `<len>"<utf-8 bytes>`. `String` and `Symbol` differ "by type, not content".
    String(String),
    /// A raw byte array: `<len>:<bytes>`.
    Bytes(Vec<u8>),
    /// A symbol: `<len>'<utf-8 bytes>`.
    Symbol(String),
    /// A sequence, `[ … ]`.
    List(Vec<Value>),
    /// A string-keyed collection, `{ … }`. Backed by a `BTreeMap` so iteration — and therefore the
    /// emitted order — is canonical and the type is deterministic.
    Struct(BTreeMap<String, Value>),
    /// A record, `< … >`. The first element conventionally multiplexes the shape (`op:deliver`),
    /// "but may be any value", so this is a plain sequence and not a keyed container.
    Record(Vec<Value>),
}

impl Value {
    /// Encode canonically into `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Value::Bool(false) => out.push(b'f'),
            Value::Bool(true) => out.push(b't'),
            Value::Int(n) => {
                let sign = match n.sign() {
                    Sign::Minus => b'-',
                    Sign::NoSign | Sign::Plus => b'+',
                };
                out.extend_from_slice(n.magnitude().to_str_radix(10).as_bytes());
                out.push(sign);
            }
            Value::Float64(f) => {
                out.push(b'D');
                out.extend_from_slice(&f.to_be_bytes());
            }
            Value::String(s) => write_len_prefixed(out, b'"', s.as_bytes()),
            Value::Symbol(s) => write_len_prefixed(out, b'\'', s.as_bytes()),
            Value::Bytes(b) => write_len_prefixed(out, b':', b),
            Value::List(xs) => write_seq(out, b'[', b']', xs),
            Value::Record(xs) => write_seq(out, b'<', b'>', xs),
            Value::Struct(m) => {
                // The reference sorts members by the *encoded key bytes*
                // (`sorted(..., key=lambda x: x[0])` on `syrup_encode(key)`), not by the key
                // string. The two orders diverge as soon as two keys differ in length — `"b"`
                // sorts before `"aa"` by encoded bytes, after it by string — so getting this wrong
                // would silently produce different bytes for the same value, and the session
                // signature covers a record that contains a struct.
                let mut pairs: Vec<(Vec<u8>, &Value)> = m
                    .iter()
                    .map(|(k, v)| (len_prefixed(b'"', k.as_bytes()), v))
                    .collect();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                out.push(b'{');
                for (encoded_key, v) in pairs {
                    out.extend_from_slice(&encoded_key);
                    v.encode_into(out);
                }
                out.push(b'}');
            }
        }
    }

    /// The canonical byte encoding.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// Decode one value and report how many bytes it used, without requiring the whole buffer.
    ///
    /// This is what a streaming netlayer needs: it frames CapTP messages by parsing one Syrup value
    /// at a time off a byte stream. A short read shows up as [`SyrupError::UnexpectedEof`] or
    /// [`SyrupError::LengthPastEnd`] — the two "need more bytes" answers — and any other error is a
    /// genuine protocol error, not a partial read.
    pub fn decode_prefix(bytes: &[u8]) -> Result<(Value, usize), SyrupError> {
        let mut r = Reader {
            bytes,
            pos: 0,
            depth: 0,
        };
        let v = r.value()?;
        Ok((v, r.pos))
    }

    /// Decode exactly one canonical value. Any trailing byte is an error, never ignored.
    pub fn from_bytes(bytes: &[u8]) -> Result<Value, SyrupError> {
        let (v, mut pos) = Self::decode_prefix(bytes)?;
        while matches!(bytes.get(pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            pos += 1;
        }
        if pos != bytes.len() {
            return Err(SyrupError::TrailingBytes(bytes.len() - pos));
        }
        Ok(v)
    }
}

fn len_prefixed(delim: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.push(delim);
    out.extend_from_slice(payload);
    out
}

fn write_len_prefixed(out: &mut Vec<u8>, delim: u8, payload: &[u8]) {
    out.extend_from_slice(&len_prefixed(delim, payload));
}

fn write_seq(out: &mut Vec<u8>, open: u8, close: u8, xs: &[Value]) {
    out.push(open);
    for x in xs {
        x.encode_into(out);
    }
    out.push(close);
}

/// How a Syrup decode can fail. Every variant is a refusal; none is a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyrupError {
    /// Input ended in the middle of a value.
    UnexpectedEof,
    /// A digit run (a length, or an integer too large for the buffer index) overflowed.
    LengthOverflow,
    /// A value did not begin with a valid token start.
    ExpectedLength,
    /// A length-prefixed payload claimed more bytes than remain.
    LengthPastEnd { claimed: u64, remaining: usize },
    /// After a length, the delimiter was not `"`, `'` or `:`.
    BadLengthDelimiter(u8),
    /// A string or symbol payload was not valid UTF-8.
    NotUtf8,
    /// An integer's digits failed to parse.
    BadInteger,
    /// Nesting exceeded [`MAX_DEPTH`].
    DepthExceeded,
    /// A struct member's key was not a String.
    StructKeyNotString,
    /// A struct named one key twice (`Model.md`: keys must be pairwise non-Equal).
    DuplicateStructKey(String),
    /// Bytes remained after a complete value.
    TrailingBytes(usize),
}

impl fmt::Display for SyrupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyrupError::UnexpectedEof => write!(f, "syrup: input ended inside a value"),
            SyrupError::LengthOverflow => write!(f, "syrup: a length or digit run overflowed"),
            SyrupError::ExpectedLength => {
                write!(f, "syrup: expected a value, found no valid start")
            }
            SyrupError::LengthPastEnd { claimed, remaining } => {
                write!(
                    f,
                    "syrup: length {claimed} exceeds the {remaining} bytes remaining"
                )
            }
            SyrupError::BadLengthDelimiter(b) => write!(f, "syrup: bad length delimiter {b:#04x}"),
            SyrupError::NotUtf8 => write!(f, "syrup: payload is not valid UTF-8 where required"),
            SyrupError::BadInteger => write!(f, "syrup: integer digits did not parse"),
            SyrupError::DepthExceeded => write!(f, "syrup: nesting exceeded {MAX_DEPTH}"),
            SyrupError::StructKeyNotString => write!(f, "syrup: struct key is not a string"),
            SyrupError::DuplicateStructKey(k) => write!(f, "syrup: struct key {k:?} appears twice"),
            SyrupError::TrailingBytes(n) => write!(f, "syrup: {n} trailing bytes after the value"),
        }
    }
}

impl std::error::Error for SyrupError {}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Consume a maximal run of ASCII digits. The run touches only `self.bytes`, so the returned
    /// slice borrows the input buffer (`'a`), not `self` — which lets callers keep working after.
    fn read_digits(&mut self) -> Result<&'a [u8], SyrupError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(SyrupError::ExpectedLength);
        }
        Ok(&self.bytes[start..self.pos])
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], SyrupError> {
        let remaining = self.bytes.len() - self.pos;
        if len > remaining {
            return Err(SyrupError::LengthPastEnd {
                claimed: len as u64,
                remaining,
            });
        }
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn value(&mut self) -> Result<Value, SyrupError> {
        self.skip_ws();
        match self.peek().ok_or(SyrupError::UnexpectedEof)? {
            b'f' => {
                self.pos += 1;
                Ok(Value::Bool(false))
            }
            b't' => {
                self.pos += 1;
                Ok(Value::Bool(true))
            }
            b'D' => {
                self.pos += 1;
                let raw = self.take(8)?;
                let mut arr = [0u8; 8];
                arr.copy_from_slice(raw);
                Ok(Value::Float64(f64::from_be_bytes(arr)))
            }
            b'[' => {
                self.pos += 1;
                Ok(Value::List(self.seq(b']')?))
            }
            b'<' => {
                self.pos += 1;
                Ok(Value::Record(self.seq(b'>')?))
            }
            b'{' => {
                self.pos += 1;
                Ok(Value::Struct(self.struct_body()?))
            }
            b if b.is_ascii_digit() => self.digit_led(),
            _ => Err(SyrupError::ExpectedLength),
        }
    }

    /// A value that begins with digits is *either* an integer (`42+`) *or* a length-prefixed
    /// payload (`5"twine`) — disambiguated by the byte after the digit run.
    fn digit_led(&mut self) -> Result<Value, SyrupError> {
        let digits = self.read_digits()?;
        let next = self.peek().ok_or(SyrupError::UnexpectedEof)?;
        self.pos += 1;
        match next {
            b'+' | b'-' => {
                let mut n = BigInt::parse_bytes(digits, 10).ok_or(SyrupError::BadInteger)?;
                if next == b'-' {
                    n = -n;
                }
                Ok(Value::Int(n))
            }
            b'"' | b'\'' | b':' => {
                let len = digits_to_u64(digits)?;
                let len = usize::try_from(len).map_err(|_| SyrupError::LengthOverflow)?;
                let payload = self.take(len)?;
                match next {
                    b'"' => Ok(Value::String(utf8(payload)?.to_owned())),
                    b'\'' => Ok(Value::Symbol(utf8(payload)?.to_owned())),
                    _ => Ok(Value::Bytes(payload.to_vec())),
                }
            }
            other => Err(SyrupError::BadLengthDelimiter(other)),
        }
    }

    /// The elements of a `[…]` or `<…>` body, up to and including `close`.
    fn seq(&mut self, close: u8) -> Result<Vec<Value>, SyrupError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(SyrupError::DepthExceeded);
        }
        let mut xs = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(SyrupError::UnexpectedEof),
                Some(b) if b == close => {
                    self.pos += 1;
                    break;
                }
                Some(_) => xs.push(self.value()?),
            }
        }
        self.depth -= 1;
        Ok(xs)
    }

    /// The `key value …` members of a `{…}` body, up to and including `}`.
    fn struct_body(&mut self) -> Result<BTreeMap<String, Value>, SyrupError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(SyrupError::DepthExceeded);
        }
        let mut m = BTreeMap::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(SyrupError::UnexpectedEof),
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(b) if b.is_ascii_digit() => {
                    let key = match self.digit_led()? {
                        Value::String(s) => s,
                        _ => return Err(SyrupError::StructKeyNotString),
                    };
                    if m.contains_key(&key) {
                        return Err(SyrupError::DuplicateStructKey(key));
                    }
                    let val = self.value()?;
                    m.insert(key, val);
                }
                Some(_) => return Err(SyrupError::StructKeyNotString),
            }
        }
        self.depth -= 1;
        Ok(m)
    }
}

fn utf8(bytes: &[u8]) -> Result<&str, SyrupError> {
    std::str::from_utf8(bytes).map_err(|_| SyrupError::NotUtf8)
}

fn digits_to_u64(digits: &[u8]) -> Result<u64, SyrupError> {
    let mut v: u64 = 0;
    for &b in digits {
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(u64::from(b - b'0')))
            .ok_or(SyrupError::LengthOverflow)?;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(v: Value) {
        let bytes = v.to_bytes();
        assert_eq!(Value::from_bytes(&bytes), Ok(v), "bytes: {bytes:?}");
    }

    #[test]
    fn syrup_bool_kat() {
        assert_eq!(Value::Bool(true).to_bytes(), b"t".to_vec());
        assert_eq!(Value::Bool(false).to_bytes(), b"f".to_vec());
        assert_eq!(Value::from_bytes(b"t"), Ok(Value::Bool(true)));
    }

    #[test]
    fn syrup_integer_kat() {
        assert_eq!(Value::Int(BigInt::from(42)).to_bytes(), b"42+".to_vec());
        assert_eq!(Value::Int(BigInt::from(0)).to_bytes(), b"0+".to_vec());
        assert_eq!(Value::Int(BigInt::from(-1)).to_bytes(), b"1-".to_vec());
        // Arbitrary precision, not u64: a 128-bit value round-trips exactly.
        let big: BigInt =
            BigInt::parse_bytes(b"340282366920938463463374607431768211456", 10).unwrap();
        assert_eq!(
            Value::Int(big).to_bytes(),
            b"340282366920938463463374607431768211456+".to_vec()
        );
    }

    #[test]
    fn syrup_string_symbol_bytes_kat() {
        assert_eq!(
            Value::String("twine".into()).to_bytes(),
            b"5\"twine".to_vec()
        );
        assert_eq!(
            Value::Symbol("fleur-de-lis".into()).to_bytes(),
            b"12'fleur-de-lis".to_vec()
        );
        let mut expected = b"8:".to_vec();
        expected.extend_from_slice(&[0xb0, 0xb5, 0xc0, 0xff, 0xee, 0xfa, 0xca, 0xde]);
        assert_eq!(
            Value::Bytes(vec![0xb0, 0xb5, 0xc0, 0xff, 0xee, 0xfa, 0xca, 0xde]).to_bytes(),
            expected
        );
    }

    #[test]
    fn syrup_float64_kat_and_round_trip() {
        let mut expected = vec![b'D'];
        expected.extend_from_slice(&f64::NAN.to_be_bytes());
        assert_eq!(Value::Float64(f64::NAN).to_bytes(), expected);
        // The spec's NaN example is big-endian 0x7ff8…
        assert_eq!(&expected[1..], &[0x7f, 0xf8, 0, 0, 0, 0, 0, 0]);
        rt(Value::Float64(-1.5));
    }

    #[test]
    fn syrup_record_kat() {
        let r = Value::Record(vec![
            Value::Symbol("foo".into()),
            Value::Int(1.into()),
            Value::Int(2.into()),
            Value::Int(3.into()),
        ]);
        // `<3'foo 1+ 2+ 3+>` in the abstract notation; canonical concrete form has no spacing.
        assert_eq!(r.to_bytes(), b"<3'foo1+2+3+>".to_vec());
        rt(r);
    }

    #[test]
    fn syrup_empty_containers_round_trip() {
        rt(Value::List(vec![]));
        rt(Value::Record(vec![]));
        rt(Value::Struct(BTreeMap::new()));
    }

    #[test]
    fn syrup_struct_keys_are_emitted_in_sorted_order() {
        let mut m = BTreeMap::new();
        m.insert("b".to_string(), Value::Int(2.into()));
        m.insert("a".to_string(), Value::Int(1.into()));
        assert_eq!(
            Value::Struct(m.clone()).to_bytes(),
            b"{1\"a1+1\"b2+}".to_vec()
        );
        rt(Value::Struct(m));
    }

    /// Members are ordered by the **encoded** key, not the key string, matching the reference's
    /// `sorted(..., key=lambda x: x[0])`. `"b"` encodes to `1"b` and `"aa"` to `2"aa"`, so `"b"`
    /// comes first — the reverse of string order. This is the case a plain `BTreeMap<String, _>`
    /// gets wrong, and the session signature covers a struct, so it is not cosmetic.
    #[test]
    fn syrup_struct_keys_are_ordered_by_encoded_bytes_not_by_string() {
        let mut m = BTreeMap::new();
        m.insert("aa".to_string(), Value::Int(1.into()));
        m.insert("b".to_string(), Value::Int(2.into()));
        assert_eq!(Value::Struct(m).to_bytes(), b"{1\"b2+2\"aa1+}".to_vec());
    }

    #[test]
    fn syrup_duplicate_struct_key_is_refused() {
        assert_eq!(
            Value::from_bytes(b"{1\"a1+1\"a2+}"),
            Err(SyrupError::DuplicateStructKey("a".into()))
        );
    }

    #[test]
    fn syrup_non_string_struct_key_is_refused() {
        assert_eq!(
            Value::from_bytes(b"{1+1+}"),
            Err(SyrupError::StructKeyNotString)
        );
    }

    #[test]
    fn syrup_rejects_trailing_bytes() {
        assert_eq!(Value::from_bytes(b"tt"), Err(SyrupError::TrailingBytes(1)));
    }

    #[test]
    fn syrup_decode_prefix_reports_consumed_bytes() {
        // Two values back to back: each prefix decode consumes exactly its own bytes.
        let stream = b"1'a<1'b1+>".to_vec();
        let (first, n) = Value::decode_prefix(&stream).unwrap();
        assert_eq!(first, Value::Symbol("a".into()));
        assert_eq!(n, 3); // `1'a`
        let (second, m) = Value::decode_prefix(&stream[n..]).unwrap();
        assert_eq!(
            second,
            Value::Record(vec![Value::Symbol("b".into()), Value::Int(1.into())])
        );
        assert_eq!(n + m, stream.len());
    }

    #[test]
    fn syrup_decode_prefix_asks_for_more_on_a_partial_value() {
        // A truncated record is "need more", not a protocol error — the netlayer relies on that.
        assert_eq!(
            Value::decode_prefix(b"<1'a"),
            Err(SyrupError::UnexpectedEof)
        );
        assert_eq!(
            Value::decode_prefix(b"12'a"),
            Err(SyrupError::LengthPastEnd {
                claimed: 12,
                remaining: 1
            })
        );
    }

    #[test]
    fn syrup_tolerates_inter_token_whitespace() {
        assert_eq!(
            Value::from_bytes(b"< 1'a 1+ >"),
            Ok(Value::Record(vec![
                Value::Symbol("a".into()),
                Value::Int(1.into())
            ]))
        );
    }

    #[test]
    fn syrup_length_past_end_is_refused_not_allocated() {
        assert_eq!(
            Value::from_bytes(b"100\"x"),
            Err(SyrupError::LengthPastEnd {
                claimed: 100,
                remaining: 1
            })
        );
    }

    #[test]
    fn syrup_bad_length_delimiter_is_refused() {
        assert_eq!(
            Value::from_bytes(b"1x"),
            Err(SyrupError::BadLengthDelimiter(b'x'))
        );
    }

    #[test]
    fn syrup_depth_bound_is_refused() {
        let deep = "[".repeat(MAX_DEPTH + 1);
        assert_eq!(
            Value::from_bytes(deep.as_bytes()),
            Err(SyrupError::DepthExceeded)
        );
    }

    #[test]
    fn syrup_unterminated_is_eof_not_panic() {
        assert_eq!(Value::from_bytes(b"["), Err(SyrupError::UnexpectedEof));
        assert_eq!(Value::from_bytes(b""), Err(SyrupError::UnexpectedEof));
        assert_eq!(Value::from_bytes(b"{1\"a"), Err(SyrupError::UnexpectedEof));
        // A length that overruns the buffer is reported as such, not merely as EOF.
        assert_eq!(
            Value::from_bytes(b"12'"),
            Err(SyrupError::LengthPastEnd {
                claimed: 12,
                remaining: 0
            })
        );
    }
}
