use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

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
}
