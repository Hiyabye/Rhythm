# University certificate-chain completion

`www.sogang.ac.kr` currently presents its leaf without its intermediate certificate.
Ordinary Rustls/WebPKI verification therefore fails with `UnknownIssuer` even though
the leaf has a valid path to a Mozilla-trusted root when the intermediate is supplied.

`sogang-intermediates.pem` contains these **public, untrusted chain candidates**:

| Certificate | SHA-256 of DER |
| --- | --- |
| Sectigo Public Server Authentication CA OV R36 | `65:42:D1:76:BE:D5:0F:19:3C:0C:E2:97:AE:44:EC:D8:A0:A8:6B:EC:2E:DE:68:27:69:34:40:59:B4:E7:85:30` |
| GoGetSSL RSA DV SSL CA 2 | `B5:28:67:96:DA:DF:16:52:1D:E4:17:72:AB:2F:DB:58:18:72:99:71:B5:27:47:21:14:6C:6E:81:72:BE:FE:07` |

Source: the user's reference collector,
[`certificates/sogang-intermediates.pem` at commit `5d53d9575c3f92ef2f36d39cecc2c8a406e5be64`](https://github.com/Hiyabye/sogang-notices/blob/5d53d9575c3f92ef2f36d39cecc2c8a406e5be64/certificates/sogang-intermediates.pem).
The bytes were fetched as data; no reference JavaScript was executed.

`tls.rs::ChainCompletion` supplies these certificates only when the TLS server name
is exactly `www.sogang.ac.kr`. It delegates certificate and handshake-signature
verification to `WebPkiServerVerifier`. They are **never added to the root store**.
The root store is exactly `webpki_roots::TLS_SERVER_ROOTS`; system trust and global
Rustls crypto-provider settings are not changed. Normal hostname, validity,
signature, certificate-usage, and trusted-root path checks remain mandatory.
No invalid-certificate acceptance or authentication bypass is used.

The two candidates accommodate the reference's known university issuers, but only
the Sectigo OV R36 candidate was needed by the live `www` leaf at verification time.
Other hosts, including Computing, receive no supplemental chain candidates. An
unknown future issuer must fail normally rather than trigger automatic CA downloads.
When maintaining these files, verify issuer/root relationships and fingerprint any
replacement; do not repair connectivity by promoting intermediates to trust anchors.

## Deterministic verification fixture

`www-leaf-test.pem` is a public leaf captured with `openssl s_client -showcerts`
from `www.sogang.ac.kr:443` on 2026-10-24. It is used **only by tests**, never as a
production trust anchor or pinned server leaf. Its properties are:

- Subject: `C=KR, ST=Seoul, O=SOGANG UNIVERSITY, CN=*.sogang.ac.kr`.
- Issuer: Sectigo Public Server Authentication CA OV R36.
- Validity: 2026-08-13 00:00:00 UTC through 2027-02-27 23:59:59 UTC.
- SHA-256: `EE:CD:35:9F:FB:9E:3F:3A:34:B8:97:43:13:89:2B:C4:DC:00:5A:96:8E:1A:2A:1A:33:2C:8A:04:F8:1F:E4:AA`.

Tests use a fixed time inside that window, not the current clock. They prove that
plain verification rejects the missing chain, exact-host completion succeeds,
other hosts receive no completion, and malformed/tampered/expired/wrong-host
certificates still fail. The ignored `live_main_api_uses_verified_completed_chain`
test performs one bounded public API GET and checks actual article JSON. It is not
part of network-free CI and does not establish historical import or offline storage.
