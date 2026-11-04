use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::Digest;

/// The QR payload from `01-architecture.md` §7:
/// `pocketai://pair?v=1&id=<host-id>&n=<urlenc name>&h=<ip1,ip2,host.local>&p=<port>&fp=<base64url sha256(cert)>&s=<base64url s>`
#[derive(Debug, Clone, PartialEq)]
pub struct PairingUri {
    pub host_id: String,
    pub host_name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub cert_fingerprint: String,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PairingUriError {
    #[error("not a pocketai:// pairing URI")]
    WrongScheme,
    #[error("unsupported version {0}")]
    UnsupportedVersion(String),
    #[error("missing required field '{0}'")]
    MissingField(&'static str),
    #[error("port '{0}' is not a valid port number")]
    InvalidPort(String),
}

pub fn build_pairing_uri(offer: &PairingUri) -> String {
    format!(
        "pocketai://pair?v=1&id={}&n={}&h={}&p={}&fp={}&s={}",
        urlencode(&offer.host_id),
        urlencode(&offer.host_name),
        urlencode(&offer.addrs.join(",")),
        offer.port,
        urlencode(&offer.cert_fingerprint),
        urlencode(&offer.secret),
    )
}

pub fn parse_pairing_uri(uri: &str) -> Result<PairingUri, PairingUriError> {
    let rest = uri
        .strip_prefix("pocketai://pair?")
        .ok_or(PairingUriError::WrongScheme)?;

    let mut v = None;
    let mut id = None;
    let mut n = None;
    let mut h = None;
    let mut p = None;
    let mut fp = None;
    let mut s = None;

    for pair in rest.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let decoded = urldecode(value);
        match key {
            "v" => v = Some(decoded),
            "id" => id = Some(decoded),
            "n" => n = Some(decoded),
            "h" => h = Some(decoded),
            "p" => p = Some(decoded),
            "fp" => fp = Some(decoded),
            "s" => s = Some(decoded),
            _ => {}
        }
    }

    let v = v.ok_or(PairingUriError::MissingField("v"))?;
    if v != "1" {
        return Err(PairingUriError::UnsupportedVersion(v));
    }
    let port_str = p.ok_or(PairingUriError::MissingField("p"))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| PairingUriError::InvalidPort(port_str))?;

    Ok(PairingUri {
        host_id: id.ok_or(PairingUriError::MissingField("id"))?,
        host_name: n.ok_or(PairingUriError::MissingField("n"))?,
        addrs: h
            .ok_or(PairingUriError::MissingField("h"))?
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        port,
        cert_fingerprint: fp.ok_or(PairingUriError::MissingField("fp"))?,
        secret: s.ok_or(PairingUriError::MissingField("s"))?,
    })
}

/// 128-bit single-use pairing secret, base64url-encoded. Security-sensitive
/// (the pinned-TLS channel plus this secret is the whole pairing auth per
/// A§7), so this is OS CSPRNG output (`getrandom`), not a hand-rolled PRNG.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// NET-23: alphabet for the typed pairing code - base32 without 0/O/1/I, so
/// nothing on the Mac's screen can be misread when typed on the phone.
const MANUAL_CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
pub const MANUAL_CODE_LEN: usize = 8;
/// Wrong tries before a manual code is dead. 40-bit code, 5 tries: a LAN
/// guesser wins with probability 5 / 2^40.
pub const MANUAL_CODE_MAX_ATTEMPTS: u32 = 5;

/// NET-23: a fresh 8-character one-time code (40 bits from the OS CSPRNG).
pub fn generate_manual_code() -> String {
    let mut bytes = [0u8; 5];
    getrandom::fill(&mut bytes).expect("OS CSPRNG must be available");
    let bits = bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
    (0..MANUAL_CODE_LEN)
        .map(|i| MANUAL_CODE_ALPHABET[((bits >> (35 - 5 * i)) & 31) as usize] as char)
        .collect()
}

/// Canonical form of a typed code: spaces and dashes dropped, uppercased,
/// exactly 8 alphabet characters. `None` if it can't be a pairing code.
pub fn normalize_manual_code(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == MANUAL_CODE_LEN && code.bytes().all(|b| MANUAL_CODE_ALPHABET.contains(&b)))
        .then_some(code)
}

/// NET-23 Short Authentication String, 6 decimal digits:
/// `SHA-256("pocketai-sas-v1" || fp[32] || code[8])`, first 4 bytes as a
/// big-endian u32, mod 10^6. `fp` is the raw 32-byte SHA-256 of the Mac's
/// leaf cert (accepted as base64url or standard base64); `code` is the
/// normalized manual code. Every field is fixed-length, so the
/// concatenation is unambiguous.
///
/// Deliberately no phone-supplied input: anything the phone sends travels
/// in the same request as the code, so a man-in-the-middle could rewrite it
/// after learning the code and grind it until the Mac's SAS matches. The
/// cert, by contrast, is fixed at the TLS handshake, before the code is
/// sent - so an attacker's cert matches the real Mac's SAS only by chance.
pub fn pairing_sas(cert_fingerprint: &str, code: &str) -> Option<String> {
    let fp = decode_fingerprint(cert_fingerprint)?;
    let code = normalize_manual_code(code)?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"pocketai-sas-v1");
    hasher.update(fp);
    hasher.update(code.as_bytes());
    let digest = hasher.finalize();
    let n = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    Some(format!("{n:06}"))
}

fn decode_fingerprint(fp: &str) -> Option<[u8; 32]> {
    let norm: String = fp
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect();
    URL_SAFE_NO_PAD.decode(norm).ok()?.try_into().ok()
}

/// NET-23: one live manual code on the Mac - 300 s, single use, and dead
/// after `MANUAL_CODE_MAX_ATTEMPTS` wrong tries.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualCode {
    pub code: String,
    pub expires_at_ms: i64,
    attempts_left: u32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ManualCodeError {
    #[error("no pairing code is active on the Mac")]
    NoCode,
    #[error("the pairing code expired")]
    Expired,
    #[error("wrong pairing code ({attempts_left} tries left)")]
    Wrong { attempts_left: u32 },
    #[error("too many wrong tries; the pairing code no longer works")]
    Exhausted,
}

impl ManualCode {
    pub fn new(code: String, expires_at_ms: i64) -> Self {
        Self {
            code,
            expires_at_ms,
            attempts_left: MANUAL_CODE_MAX_ATTEMPTS,
        }
    }
}

/// Checks `input` against the slot's code. A match takes the code out of
/// the slot (single use); expiry or the last wrong try empties it too.
pub fn redeem_manual_code(
    slot: &mut Option<ManualCode>,
    input: &str,
    now_ms: i64,
) -> Result<ManualCode, ManualCodeError> {
    let current = slot.as_mut().ok_or(ManualCodeError::NoCode)?;
    if now_ms >= current.expires_at_ms {
        *slot = None;
        return Err(ManualCodeError::Expired);
    }
    let typed = normalize_manual_code(input).unwrap_or_default();
    if constant_time_eq(typed.as_bytes(), current.code.as_bytes()) {
        return Ok(slot.take().expect("checked above"));
    }
    current.attempts_left -= 1;
    if current.attempts_left == 0 {
        *slot = None;
        return Err(ManualCodeError::Exhausted);
    }
    Err(ManualCodeError::Wrong {
        attempts_left: current.attempts_left,
    })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PairingUri {
        PairingUri {
            host_id: "01JHOST".into(),
            host_name: "Rohit's Mac".into(),
            addrs: vec!["192.168.1.5".into(), "Rohits-Mac.local".into()],
            port: 7480,
            cert_fingerprint: "deadbeef_-fp".into(),
            secret: generate_secret(),
        }
    }

    #[test]
    fn round_trips_through_build_and_parse() {
        let offer = sample();
        let uri = build_pairing_uri(&offer);
        assert!(uri.starts_with("pocketai://pair?v=1&"));
        let parsed = parse_pairing_uri(&uri).unwrap();
        assert_eq!(parsed, offer);
    }

    #[test]
    fn urlencodes_the_display_name_with_spaces_and_apostrophe() {
        let uri = build_pairing_uri(&sample());
        assert!(uri.contains("n=Rohit%27s%20Mac"));
    }

    #[test]
    fn rejects_wrong_scheme() {
        assert_eq!(
            parse_pairing_uri("https://example.com"),
            Err(PairingUriError::WrongScheme)
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        let bad = "pocketai://pair?v=2&id=x&n=x&h=x&p=7480&fp=x&s=x";
        assert_eq!(
            parse_pairing_uri(bad),
            Err(PairingUriError::UnsupportedVersion("2".into()))
        );
    }

    #[test]
    fn rejects_missing_field() {
        let bad = "pocketai://pair?v=1&id=x&n=x&h=x&p=7480&fp=x";
        assert_eq!(
            parse_pairing_uri(bad),
            Err(PairingUriError::MissingField("s"))
        );
    }

    #[test]
    fn rejects_garbage_port() {
        let bad = "pocketai://pair?v=1&id=x&n=x&h=x&p=notaport&fp=x&s=x";
        assert!(matches!(
            parse_pairing_uri(bad),
            Err(PairingUriError::InvalidPort(_))
        ));
    }

    #[test]
    fn generated_secrets_are_16_bytes_and_unique() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a, b);
        assert_eq!(URL_SAFE_NO_PAD.decode(&a).unwrap().len(), 16);
    }
    // ---- NET-23 manual code + SAS ----

    const FP: &str = "q83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4k"; // 32 bytes b64url

    #[test]
    fn manual_codes_are_8_unambiguous_chars_and_unique() {
        let a = generate_manual_code();
        assert_eq!(a.len(), 8);
        assert!(a.bytes().all(|b| MANUAL_CODE_ALPHABET.contains(&b)));
        assert!(!a.contains(['0', 'O', '1', 'I']));
        assert_ne!(a, generate_manual_code());
    }

    #[test]
    fn normalize_accepts_spacing_dashes_and_lowercase_only() {
        assert_eq!(
            normalize_manual_code("abcd-efgh").as_deref(),
            Some("ABCDEFGH")
        );
        assert_eq!(
            normalize_manual_code(" AB CD EF GH ").as_deref(),
            Some("ABCDEFGH")
        );
        assert_eq!(normalize_manual_code("ABCDEFG"), None); // short
        assert_eq!(normalize_manual_code("ABCDEFG0"), None); // '0' not in alphabet
    }

    #[test]
    fn same_inputs_give_the_same_six_digit_sas() {
        let a = pairing_sas(FP, "ABCD-EFGH").unwrap();
        assert_eq!(a.len(), 6);
        assert!(a.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(Some(a.clone()), pairing_sas(FP, "abcdefgh"));
        // The phone may hand over standard base64 with padding; same SAS.
        let raw = URL_SAFE_NO_PAD.decode(FP).unwrap();
        let std_b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        assert_eq!(Some(a), pairing_sas(&std_b64, "ABCDEFGH"));
    }

    #[test]
    fn a_different_fingerprint_or_code_changes_the_sas() {
        let a = pairing_sas(FP, "ABCDEFGH").unwrap();
        let other_fp = URL_SAFE_NO_PAD.encode([7u8; 32]);
        assert_ne!(Some(a.clone()), pairing_sas(&other_fp, "ABCDEFGH"));
        assert_ne!(Some(a), pairing_sas(FP, "ABCDEFGJ"));
    }

    #[test]
    fn sas_rejects_malformed_inputs() {
        assert_eq!(pairing_sas("not-a-fingerprint", "ABCDEFGH"), None);
        assert_eq!(
            pairing_sas(&URL_SAFE_NO_PAD.encode([1u8; 16]), "ABCDEFGH"),
            None
        );
        assert_eq!(pairing_sas(FP, "short"), None);
    }

    #[test]
    fn manual_code_is_single_use() {
        let mut slot = Some(ManualCode::new("ABCDEFGH".into(), 1_000));
        assert!(redeem_manual_code(&mut slot, "abcd-efgh", 10).is_ok());
        assert_eq!(slot, None);
        assert_eq!(
            redeem_manual_code(&mut slot, "ABCDEFGH", 10),
            Err(ManualCodeError::NoCode)
        );
    }

    #[test]
    fn manual_code_expires() {
        let mut slot = Some(ManualCode::new("ABCDEFGH".into(), 1_000));
        assert_eq!(
            redeem_manual_code(&mut slot, "ABCDEFGH", 1_000),
            Err(ManualCodeError::Expired)
        );
        assert_eq!(slot, None);
    }

    #[test]
    fn manual_code_dies_after_five_wrong_tries() {
        let mut slot = Some(ManualCode::new("ABCDEFGH".into(), 1_000));
        for left in (1..MANUAL_CODE_MAX_ATTEMPTS).rev() {
            assert_eq!(
                redeem_manual_code(&mut slot, "ZZZZZZZZ", 10),
                Err(ManualCodeError::Wrong {
                    attempts_left: left
                })
            );
        }
        assert_eq!(
            redeem_manual_code(&mut slot, "garbage", 10),
            Err(ManualCodeError::Exhausted)
        );
        // Even the right code is useless now.
        assert_eq!(
            redeem_manual_code(&mut slot, "ABCDEFGH", 10),
            Err(ManualCodeError::NoCode)
        );
    }
}
