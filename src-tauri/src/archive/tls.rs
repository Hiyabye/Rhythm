//! Complete a known missing server chain without adding certificate trust anchors.
use base64::{engine::general_purpose::STANDARD, Engine};
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        WebPkiServerVerifier,
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};
use std::sync::Arc;

const INTERMEDIATES: &str = include_str!("certificates/sogang-intermediates.pem");

#[derive(Debug)]
struct ChainCompletion {
    standard: Arc<WebPkiServerVerifier>,
    supplemental: Vec<CertificateDer<'static>>,
}

impl ServerCertVerifier for ChainCompletion {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if matches!(server_name, ServerName::DnsName(name) if name.as_ref() == "www.sogang.ac.kr") {
            let mut chain = intermediates.to_vec();
            for certificate in &self.supplemental {
                if !chain.contains(certificate) {
                    chain.push(certificate.clone());
                }
            }
            // These are untrusted chain candidates, never roots. WebPKI still checks
            // signatures, constraints, validity, hostname, and the Mozilla trust path.
            self.standard
                .verify_server_cert(end_entity, &chain, server_name, ocsp_response, now)
        } else {
            self.standard.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            )
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.standard
            .verify_tls12_signature(message, cert, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.standard
            .verify_tls13_signature(message, cert, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.standard.supported_verify_schemes()
    }
}

fn roots() -> RootCertStore {
    RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
}

fn verifier() -> Result<Arc<ChainCompletion>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let standard = WebPkiServerVerifier::builder_with_provider(Arc::new(roots()), provider)
        .build()
        .map_err(|_| "HTTPS 인증서 검증을 준비하지 못했습니다.".to_string())?;
    let supplemental = decode_pem(INTERMEDIATES)?;
    if supplemental.len() != 2 {
        return Err("학교 서버의 중간 인증서 구성을 확인해 주세요.".into());
    }
    Ok(Arc::new(ChainCompletion {
        standard,
        supplemental,
    }))
}

/// Reqwest must use this exact rustls version's ClientConfig. No global crypto
/// provider, operating-system store, or application-wide trust setting is changed.
pub fn config() -> Result<ClientConfig, String> {
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| "안전한 TLS 프로토콜을 준비하지 못했습니다.".to_string())?;
    Ok(builder
        .dangerous()
        .with_custom_certificate_verifier(verifier()?)
        .with_no_client_auth())
}

fn decode_pem(pem: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    if pem.len() > 32 * 1024 {
        return Err("중간 인증서 파일이 너무 큽니다.".into());
    }
    let mut certificates = Vec::new();
    let mut rest = pem.trim();
    while !rest.is_empty() {
        let body = rest
            .strip_prefix("-----BEGIN CERTIFICATE-----")
            .ok_or("중간 인증서 형식이 올바르지 않습니다.")?;
        let (body, remaining) = body
            .split_once("-----END CERTIFICATE-----")
            .ok_or("중간 인증서가 완전하지 않습니다.")?;
        let encoded: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let der = STANDARD
            .decode(encoded)
            .map_err(|_| "중간 인증서 인코딩이 올바르지 않습니다.".to_string())?;
        if der.is_empty() || certificates.len() >= 8 {
            return Err("중간 인증서 구성이 올바르지 않습니다.".into());
        }
        certificates.push(CertificateDer::from(der));
        rest = remaining.trim();
    }
    Ok(certificates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::time::Duration;

    fn leaf() -> CertificateDer<'static> {
        decode_pem(include_str!("certificates/www-leaf-test.pem"))
            .unwrap()
            .remove(0)
    }
    fn valid_time() -> UnixTime {
        // Fixed inside the fixture leaf's 2026-08-13 to 2027-02-27 validity window.
        UnixTime::since_unix_epoch(Duration::from_secs(1_792_800_000))
    }

    #[test]
    fn pinned_intermediates_do_not_expand_mozilla_roots() {
        let certificates = decode_pem(INTERMEDIATES).unwrap();
        let expected = [
            "6542d176bed50f193c0ce297ae44ecd8a0a86bec2ede682769344059b4e78530",
            "b5286796dadf16521de41772ab2fdb5818729971b5274721146c6e8172befe07",
        ];
        assert_eq!(certificates.len(), expected.len());
        for (certificate, fingerprint) in certificates.iter().zip(expected) {
            assert_eq!(
                format!("{:x}", Sha256::digest(certificate.as_ref())),
                fingerprint
            );
        }
        assert_eq!(roots().roots, webpki_roots::TLS_SERVER_ROOTS);
        assert!(config().is_ok());
    }

    #[test]
    fn missing_chain_is_completed_only_for_exact_university_host() {
        let verifier = verifier().unwrap();
        let name = ServerName::try_from("www.sogang.ac.kr").unwrap();
        assert!(verifier
            .standard
            .verify_server_cert(&leaf(), &[], &name, &[], valid_time())
            .is_err());
        assert!(verifier
            .verify_server_cert(&leaf(), &[], &name, &[], valid_time())
            .is_ok());
        // The wildcard leaf also covers computing, but no chain is supplied for it.
        let other = ServerName::try_from("computing.sogang.ac.kr").unwrap();
        assert!(verifier
            .verify_server_cert(&leaf(), &[], &other, &[], valid_time())
            .is_err());
        assert!(verifier
            .verify_server_cert(&leaf(), &verifier.supplemental, &other, &[], valid_time())
            .is_ok());
    }

    #[test]
    fn hostname_expiry_encoding_and_tampered_signatures_still_fail() {
        let verifier = verifier().unwrap();
        let name = ServerName::try_from("www.sogang.ac.kr").unwrap();
        let wrong = ServerName::try_from("wrong.example").unwrap();
        assert!(verifier
            .verify_server_cert(&leaf(), &verifier.supplemental, &wrong, &[], valid_time())
            .is_err());
        let expired = UnixTime::since_unix_epoch(Duration::from_secs(1_893_456_000));
        assert!(verifier
            .verify_server_cert(&leaf(), &[], &name, &[], expired)
            .is_err());
        assert!(verifier
            .verify_server_cert(
                &CertificateDer::from(vec![1, 2, 3]),
                &[],
                &name,
                &[],
                valid_time()
            )
            .is_err());
        let mut tampered = leaf().as_ref().to_vec();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(verifier
            .verify_server_cert(
                &CertificateDer::from(tampered),
                &[],
                &name,
                &[],
                valid_time()
            )
            .is_err());
    }

    #[tokio::test]
    #[ignore = "bounded live check; requires university network access"]
    async fn live_main_api_uses_verified_completed_chain() {
        use super::super::network::{Client, Fetch};
        use std::sync::atomic::AtomicBool;
        let url = url::Url::parse("https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData?pkId=551637")
            .unwrap();
        let response = Client::new()
            .unwrap()
            .get(&url, None, 1024 * 1024, &AtomicBool::new(false))
            .await
            .unwrap();
        let Fetch::Body { bytes, .. } = response else {
            panic!("expected JSON document")
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["data"]["pkId"], 551637);
        assert!(value["data"]["content"]
            .as_str()
            .unwrap()
            .contains("강의평가"));
    }
}
