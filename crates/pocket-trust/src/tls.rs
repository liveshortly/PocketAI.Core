use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use rcgen::{CertificateParams, DistinguishedName, KeyPair, SanType};
use sha2::{Digest, Sha256};

pub struct TlsIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint_b64url: String,
}

/// Formats a DER-encoded certificate into standard PEM format.
pub fn der_to_pem(cert_der: &[u8]) -> String {
    let b64 = STANDARD.encode(cert_der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).expect("valid utf8"));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

/// Generates an ECDSA P-256 self-signed identity: SAN = hostname.local, each
/// current IP, and "pocketai.local"; validity 10 years. Fingerprint is the
/// base64url SHA-256 of the DER-encoded certificate.
pub fn generate_identity(hostname_local: &str, ips: &[std::net::IpAddr]) -> TlsIdentity {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");

    let mut sans = vec![
        SanType::DnsName(hostname_local.try_into().expect("valid dns name")),
        SanType::DnsName("pocketai.local".try_into().expect("valid dns name")),
    ];
    sans.extend(ips.iter().map(|ip| SanType::IpAddress(*ip)));

    let mut params = CertificateParams::new(Vec::new()).expect("cert params");
    params.subject_alt_names = sans;
    params.distinguished_name = DistinguishedName::new();
    params.not_before = rcgen::date_time_ymd(2026, 1, 1);
    params.not_after = rcgen::date_time_ymd(2036, 1, 1);

    let cert = params.self_signed(&key_pair).expect("self sign");
    let cert_der = cert.der().to_vec();
    let key_der = key_pair.serialize_der();

    let mut hasher = Sha256::new();
    hasher.update(&cert_der);
    let fingerprint_b64url = URL_SAFE_NO_PAD.encode(hasher.finalize());

    TlsIdentity {
        cert_der,
        key_der,
        fingerprint_b64url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    #[test]
    fn fingerprint_is_stable_sha256_of_der() {
        let id = generate_identity("Rohits-Mac.local", &[]);
        let mut hasher = Sha256::new();
        hasher.update(&id.cert_der);
        let recomputed = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(id.fingerprint_b64url, recomputed);
    }

    #[test]
    fn openssl_reports_the_same_fingerprint() {
        let Ok(which) = Command::new("which").arg("openssl").output() else {
            eprintln!("skipping: `which` unavailable");
            return;
        };
        if !which.status.success() {
            eprintln!("skipping: openssl not installed");
            return;
        }

        let id = generate_identity("Rohits-Mac.local", &[]);
        let dir = tempfile::tempdir().unwrap();
        let der_path = dir.path().join("cert.der");
        std::fs::write(&der_path, &id.cert_der).unwrap();

        let output = Command::new("openssl")
            .args(["x509", "-inform", "der", "-in"])
            .arg(&der_path)
            .args(["-noout", "-fingerprint", "-sha256"])
            .output()
            .expect("run openssl");
        assert!(output.status.success(), "openssl failed: {:?}", output);

        let stdout = String::from_utf8_lossy(&output.stdout);
        // "sha256 Fingerprint=AA:BB:...:CC"
        let hex_colon = stdout.trim().rsplit('=').next().unwrap();
        let hex: String = hex_colon.split(':').collect();
        let raw = hex::decode(&hex).unwrap();
        let openssl_fp_b64url = URL_SAFE_NO_PAD.encode(&raw);

        assert_eq!(id.fingerprint_b64url, openssl_fp_b64url);

        let _ = std::io::stdout().write_all(stdout.as_bytes());
    }
}
