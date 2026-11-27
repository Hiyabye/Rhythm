# University certificate-chain completion

These hosts send their leaf certificate without its intermediate:

- `www.sogang.ac.kr`
- `new-cms.sogang.ac.kr`
- `eclass.sogang.ac.kr`
- `scec.sogang.ac.kr`
- `klec.sogang.ac.kr`
- `admission.sogang.ac.kr`

Plain Rustls/WebPKI verification fails with `UnknownIssuer`. The leaf has a valid path to a Mozilla-trusted root once the intermediate is supplied.

## Intermediate candidates

`sogang-intermediates.pem` holds two **public, untrusted chain candidates**:

| Certificate | SHA-256 of DER |
| --- | --- |
| Sectigo Public Server Authentication CA OV R36 | `65:42:D1:76:BE:D5:0F:19:3C:0C:E2:97:AE:44:EC:D8:A0:A8:6B:EC:2E:DE:68:27:69:34:40:59:B4:E7:85:30` |
| GoGetSSL RSA DV SSL CA 2 | `B5:28:67:96:DA:DF:16:52:1D:E4:17:72:AB:2F:DB:58:18:72:99:71:B5:27:47:21:14:6C:6E:81:72:BE:FE:07` |

Source: the user's reference collector, [`certificates/sogang-intermediates.pem` at commit `5d53d9575c3f92ef2f36d39cecc2c8a406e5be64`](https://github.com/Hiyabye/sogang-notices/blob/5d53d9575c3f92ef2f36d39cecc2c8a406e5be64/certificates/sogang-intermediates.pem). The bytes were fetched as data. No reference JavaScript was run.

### How they are used

`tls.rs::ChainCompletion` supplies these certificates only when the TLS server name is exactly one of the six hosts above (`tls.rs::COMPLETED_HOSTS`). Certificate and handshake-signature verification stay with `WebPkiServerVerifier`.

- The candidates are **never added to the root store**. The root store is exactly `webpki_roots::TLS_SERVER_ROOTS`.
- System trust and global Rustls crypto-provider settings are unchanged.
- Hostname, validity, signature, certificate-usage and trusted-root path checks stay mandatory.
- No invalid certificate is accepted and no authentication is bypassed.

### Observations

- The dormitory CMS leaf (2026-11-05) and the eClass (Cybercampus) leaf (2026-11-06) are `*.sogang.ac.kr` leaves served without their intermediate. Both come from the same Sectigo OV R36 intermediate (same SHA-256), so the candidate file did not change.
- `scec` (미래교육원), `klec` (한국어교육원) and `admission` (입학처) serve the same `*.sogang.ac.kr` leaf without its intermediate (observed 2026-11-24). `openssl verify` with hostname checking accepted all three leaves through the pinned R36 candidate and `sectigo-r46-root.pem`.
- The two candidates cover the reference's known university issuers. Only the Sectigo OV R36 candidate was needed by the live `www` leaf when this was verified.
- Other hosts, including Computing, get no supplemental chain.

An unknown future issuer must fail normally. Do not download CAs automatically. When you maintain these files, verify issuer and root relationships and fingerprint any replacement. Never fix connectivity by promoting an intermediate to a trust anchor.

## Studium seat-server root

`libseat.sogang.ac.kr:444` offers only TLS 1.2 CBC suites (`ECDHE-RSA-AES128-SHA`, `AES128-SHA`, `AES256-SHA`, observed 2026-11-09). Rustls does not implement them. This origin alone uses OS TLS (`transport.rs`, `Access::Studium`, behind the crate's `studium` feature).

The server sends its complete chain: the `*.sogang.ac.kr` leaf and the same Sectigo OV R36 intermediate as above. Its OS root store is replaced by `sectigo-r46-root.pem`. A root that a user, employer or other software installed therefore cannot vouch for the seat server.

| Certificate | SHA-256 of DER |
| --- | --- |
| Sectigo Public Server Authentication Root R46 (valid until 2046-03-21) | `7B:B6:47:A6:2A:EE:AC:88:BF:25:7A:A5:22:D0:1F:FE:A3:95:E0:AB:45:C7:3F:93:F6:56:54:EC:38:F2:5A:06` |

Source: exported from the macOS system root keychain on 2026-11-09. The fingerprint matches the Mozilla-derived `webpki-roots` 1.0.11 entry. `openssl verify` accepted the intermediate the server sent against it.

- **macOS:** Secure Transport trusts only this anchor.
- **Windows:** Schannel first validates against the system store. The chain must then contain this certificate.
- Hostname, validity and signature checks stay on.

If the school moves to another CA, seat checks fail closed until this file is reviewed and replaced. Never fall back to the OS store or an intermediate anchor.

## Deterministic verification fixture

`www-leaf-test.pem` is a public leaf captured with `openssl s_client -showcerts` from `www.sogang.ac.kr:443` on 2026-10-24. **Tests only.** It is never a production trust anchor or a pinned server leaf.

- Subject: `C=KR, ST=Seoul, O=SOGANG UNIVERSITY, CN=*.sogang.ac.kr`
- Issuer: Sectigo Public Server Authentication CA OV R36
- Validity: 2026-08-13 00:00:00 UTC through 2027-02-27 23:59:59 UTC
- SHA-256: `EE:CD:35:9F:FB:9E:3F:3A:34:B8:97:43:13:89:2B:C4:DC:00:5A:96:8E:1A:2A:1A:33:2C:8A:04:F8:1F:E4:AA`

Tests use a fixed time inside that window, not the current clock. They show that:

- plain verification rejects the missing chain;
- completion succeeds for each exact allowlisted host;
- other hosts get no completion;
- malformed, tampered, expired and wrong-host certificates still fail.

The ignored `live_main_api_uses_verified_completed_chain` test makes one bounded public API GET and checks real article JSON. It is not part of network-free CI. It does not show historical import or offline storage.
