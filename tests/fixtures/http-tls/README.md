# Local HTTP TLS fixture

This is public, test-only key material. Never use this identity for a deployed
service or install its certificate in an operating-system trust store.
`identity.p12.hex` is a hex-encoded PKCS#12 identity with password `fixture`.
The certificate is self-signed, valid from September 16, 2026 through December
14, 2028, and has only the `localhost` DNS subject alternative name. Renew it
before expiry; an IP-address URL deliberately fails hostname verification.

**Keep the validity period at or under 825 days.** macOS refuses a TLS server
certificate whose validity exceeds that, failing the handshake with *"The
validity period in the certificate exceeds the maximum allowed"* (Security
framework, −67901), so a longer window makes the tests fail on macOS and
nowhere else. The previous fixture used 3650 days and did exactly that; it went
unnoticed because no lane ran these tests on macOS. `-days 820` leaves margin
below the limit while keeping renewal infrequent.

The Rust unit tests decode the identity in memory and add the certificate only
to their explicitly trusting client. The ordinary public constructor must reject
it. No external website, Python process, or OpenSSL executable is needed to run
the tests. The test server uses the same native TLS library already selected by
the HTTP backend; its direct dependency is test-only.

To regenerate in a temporary directory with OpenSSL:

```sh
openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem \
  -days 820 -subj /CN=kernal-http-test-only \
  -addext subjectAltName=DNS:localhost \
  -addext basicConstraints=critical,CA:TRUE \
  -addext keyUsage=critical,digitalSignature,keyEncipherment,keyCertSign \
  -addext extendedKeyUsage=serverAuth
openssl pkcs12 -export -inkey key.pem -in cert.pem -out identity.p12 \
  -passout pass:fixture -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES \
  -macalg sha1
od -An -v -tx1 identity.p12
```

Copy the resulting certificate and hex bytes into these fixtures and update the
validity dates above. The legacy PKCS#12 envelope is for native importer
compatibility, not a cryptographic recommendation; the RSA certificate and
negotiated TLS connection use the platform's normal verification defaults.
