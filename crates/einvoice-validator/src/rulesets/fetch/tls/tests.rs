//! The provider: known answers for the hashes, HMAC and signature verifications, and
//! full TLS handshakes in memory, client and server both on the provider, with every
//! cipher suite, key exchange group and server key type, against a synthetic test PKI
//! (testdata/, made by testdata/generate.sh).

use std::io::{Read as _, Write as _};

use rustls::client::ClientConnection;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::server::ServerConnection;
use rustls::sign::{CertifiedKey, Signer, SingleCertAndKey};
use rustls::{ClientConfig, RootCertStore, ServerConfig, SignatureAlgorithm};

use super::*;

const CA_EC: &[u8] = include_bytes!("testdata/ca-ec.der");
const EE_EC: &[u8] = include_bytes!("testdata/ee-ec.der");
const EE_EC_KEY: &[u8] = include_bytes!("testdata/ee-ec.key");
const CA_RSA: &[u8] = include_bytes!("testdata/ca-rsa.der");
const EE_RSA: &[u8] = include_bytes!("testdata/ee-rsa.der");
const EE_RSA_KEY: &[u8] = include_bytes!("testdata/ee-rsa.key");
const MESSAGE: &[u8] = include_bytes!("testdata/message.txt");
const RSA_PUBLIC: &[u8] = include_bytes!("testdata/rsa-public.der");
const EC_PUBLIC: &[u8] = include_bytes!("testdata/ec-public.bin");

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[test]
fn hashes_and_hmac_give_the_known_answers() {
    use rustls::crypto::hash::Hash as _;
    use rustls::crypto::hmac::Hmac as _;
    // FIPS 180-2 "abc".
    assert_eq!(
        hex(SHA256.hash(b"abc").as_ref()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let mut context = SHA384.start();
    context.update(b"a");
    let fork = context.fork();
    context.update(b"bc");
    assert_eq!(
        hex(context.fork_finish().as_ref()),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
         8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(context.finish().as_ref(), SHA384.hash(b"abc").as_ref());
    assert_eq!(fork.finish().as_ref(), SHA384.hash(b"a").as_ref());
    // RFC 4231, test case 2, in three pieces.
    let tag = HMAC_SHA256.with_key(b"Jefe").sign_concat(
        b"what do ya ",
        &[b"want ", b"for "],
        b"nothing?",
    );
    assert_eq!(
        hex(tag.as_ref()),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    let tag = HMAC_SHA384
        .with_key(b"Jefe")
        .sign(&[b"what do ya want for nothing?"]);
    assert_eq!(
        hex(tag.as_ref()),
        "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e\
         8e2240ca5e69e2c78b3239ecfab21649"
    );
    // RFC 4231, test case 6: a key longer than the block.
    let tag = HMAC_SHA256
        .with_key(&[0xaa; 131])
        .sign(&[b"Test Using Larger Than Block-Size Key - Hash Key First"]);
    assert_eq!(
        hex(tag.as_ref()),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

#[test]
fn verifies_the_signatures_openssl_made_and_nothing_else() {
    let cases: [(&Verify, &[u8], &[u8]); 8] = [
        (
            &RSA_PKCS1_SHA256,
            RSA_PUBLIC,
            include_bytes!("testdata/rsa-pkcs1-sha256.sig"),
        ),
        (
            &RSA_PKCS1_SHA384,
            RSA_PUBLIC,
            include_bytes!("testdata/rsa-pkcs1-sha384.sig"),
        ),
        (
            &RSA_PKCS1_SHA512,
            RSA_PUBLIC,
            include_bytes!("testdata/rsa-pkcs1-sha512.sig"),
        ),
        (
            &RSA_PSS_SHA256,
            RSA_PUBLIC,
            include_bytes!("testdata/rsa-pss-sha256.sig"),
        ),
        (
            &RSA_PSS_SHA384,
            RSA_PUBLIC,
            include_bytes!("testdata/rsa-pss-sha384.sig"),
        ),
        (
            &RSA_PSS_SHA512,
            RSA_PUBLIC,
            include_bytes!("testdata/rsa-pss-sha512.sig"),
        ),
        (
            &ECDSA_P256_SHA256,
            EC_PUBLIC,
            include_bytes!("testdata/ecdsa-p256-sha256.sig"),
        ),
        (
            &ECDSA_P256_SHA384,
            EC_PUBLIC,
            include_bytes!("testdata/ecdsa-p256-sha384.sig"),
        ),
    ];
    for (i, (algorithm, key, signature)) in cases.iter().enumerate() {
        assert!(
            algorithm.verify_signature(key, MESSAGE, signature).is_ok(),
            "{algorithm:?}"
        );
        // Another message, a damaged signature, another key: refused.
        assert!(
            algorithm
                .verify_signature(key, b"another message", signature)
                .is_err()
        );
        let mut damaged = signature.to_vec();
        let middle = damaged.len() / 2;
        damaged[middle] ^= 1;
        assert!(algorithm.verify_signature(key, MESSAGE, &damaged).is_err());
        let other = if *key == RSA_PUBLIC {
            EC_PUBLIC
        } else {
            RSA_PUBLIC
        };
        assert!(
            algorithm
                .verify_signature(other, MESSAGE, signature)
                .is_err()
        );
        // Each signature verifies only under its own algorithm.
        for (j, (other, _, _)) in cases.iter().enumerate() {
            if i != j {
                assert!(
                    other.verify_signature(key, MESSAGE, signature).is_err(),
                    "{algorithm:?} signature accepted by {other:?}"
                );
            }
        }
    }
    // The P-384 verifications refuse a P-256 key.
    let signature = include_bytes!("testdata/ecdsa-p256-sha384.sig");
    assert!(
        ECDSA_P384_SHA384
            .verify_signature(EC_PUBLIC, MESSAGE, signature)
            .is_err()
    );
}

#[test]
fn refuses_rsa_keys_under_2048_bits() {
    use rsa::pkcs1::EncodeRsaPublicKey as _;
    // A 1024-bit modulus (any odd number of that size will do for the size check).
    let mut n = vec![0xffu8; 128];
    n[127] = 0xfd;
    let key = rsa::PublicKey::new_with_size_limits(
        rsa::BoxedUint::from_be_slice(&n, 1024).unwrap(),
        rsa::BoxedUint::from(65_537u32),
        1024..=8192,
    )
    .unwrap();
    let der = key.to_pkcs1_der().unwrap();
    assert!(rsa_key(der.as_bytes()).is_err());
    assert!(rsa_key(RSA_PUBLIC).is_ok());
}

#[test]
fn key_exchange_agrees_and_refuses_bad_points() {
    for group in KX_GROUPS {
        let a = group.start().unwrap();
        let b = group.start().unwrap();
        let (a_public, b_public) = (a.pub_key().to_vec(), b.pub_key().to_vec());
        assert_eq!(a_public.first(), Some(&4), "{group:?}: uncompressed");
        let left = a.complete(&b_public).unwrap();
        let right = b.complete(&a_public).unwrap();
        assert_eq!(left.secret_bytes(), right.secret_bytes());
        assert_eq!(left.secret_bytes().len(), (a_public.len() - 1) / 2);
        // Not on the curve, compressed, empty, the wrong length: refused.
        let mut off_curve = b_public.clone();
        let last = off_curve.len() - 1;
        off_curve[last] ^= 1;
        let mut compressed = b_public[..=b_public.len() / 2].to_vec();
        compressed[0] = 2;
        for bad in [off_curve, compressed, Vec::new(), b_public[..10].to_vec()] {
            let c = group.start().unwrap();
            assert!(c.complete(&bad).is_err(), "{group:?}: {}", hex(&bad));
        }
    }
}

/// A random source for the RSA-PSS signatures of the test server.
struct TestRng;

impl rsa::rand_core::TryRng for TestRng {
    type Error = getrandom::Error;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        getrandom::u64()
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        getrandom::fill(dst)
    }
}

impl rsa::rand_core::TryCryptoRng for TestRng {}

/// The test server's key: ECDSA P-256, or RSA with its schemes in order of preference.
#[derive(Debug, Clone)]
enum ServerKey {
    Ec(p256::ecdsa::SigningKey),
    Rsa(rsa::PrivateKey, &'static [SignatureScheme]),
}

impl rustls::sign::SigningKey for ServerKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        let schemes: &[SignatureScheme] = match self {
            Self::Ec(_) => &[SignatureScheme::ECDSA_NISTP256_SHA256],
            Self::Rsa(_, schemes) => schemes,
        };
        let scheme = schemes.iter().find(|s| offered.contains(s))?;
        Some(Box::new(ServerSigner(self.clone(), *scheme)))
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        match self {
            Self::Ec(_) => SignatureAlgorithm::ECDSA,
            Self::Rsa(..) => SignatureAlgorithm::RSA,
        }
    }
}

#[derive(Debug)]
struct ServerSigner(ServerKey, SignatureScheme);

impl Signer for ServerSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, Error> {
        use rsa::signature::{RandomizedSigner as _, SignatureEncoding as _, Signer as _};
        Ok(match (&self.0, self.1) {
            (ServerKey::Ec(key), _) => {
                let signature: p256::ecdsa::Signature = key.sign(message);
                signature.to_der().as_bytes().to_vec()
            }
            (ServerKey::Rsa(key, _), SignatureScheme::RSA_PSS_SHA256) => {
                rsa::pss::SigningKey::<Sha256>::new(key.clone())
                    .try_sign_with_rng(&mut TestRng, message)
                    .unwrap()
                    .to_vec()
            }
            (ServerKey::Rsa(key, _), SignatureScheme::RSA_PKCS1_SHA256) => {
                rsa::pkcs1v15::SigningKey::<Sha256>::new(key.clone())
                    .sign(message)
                    .to_vec()
            }
            (_, scheme) => return Err(Error::General(format!("{scheme:?} not signed here"))),
        })
    }

    fn scheme(&self) -> SignatureScheme {
        self.1
    }
}

fn ec_key() -> ServerKey {
    ServerKey::Ec(p256::ecdsa::SigningKey::from_slice(EE_EC_KEY).unwrap())
}

fn rsa_key_pair(schemes: &'static [SignatureScheme]) -> ServerKey {
    use rsa::pkcs1::DecodeRsaPrivateKey as _;
    ServerKey::Rsa(
        rsa::PrivateKey::from_pkcs1_der(EE_RSA_KEY).unwrap(),
        schemes,
    )
}

/// A provider restricted to one cipher suite and one key exchange group.
fn restricted(suite: SupportedCipherSuite, group: &'static dyn SupportedKxGroup) -> CryptoProvider {
    CryptoProvider {
        cipher_suites: vec![suite],
        kx_groups: vec![group],
        ..provider()
    }
}

fn client_config(provider: CryptoProvider, root: &[u8]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(root.to_vec())).unwrap();
    Arc::new(
        ClientConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn server_config(provider: CryptoProvider, chain: &[&[u8]], key: ServerKey) -> Arc<ServerConfig> {
    let chain = chain
        .iter()
        .map(|c| CertificateDer::from(c.to_vec()))
        .collect();
    let certified = CertifiedKey::new(chain, Arc::new(key));
    Arc::new(
        ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(SingleCertAndKey::from(certified))),
    )
}

/// Moves the TLS bytes `from` has to send to `to`, and has `to` process them.
/// Returns the application data `to` received.
fn transfer<A, B, X, Y>(from: &mut A, to: &mut B) -> Result<Vec<u8>, Error>
where
    A: std::ops::DerefMut<Target = rustls::ConnectionCommon<X>>,
    B: std::ops::DerefMut<Target = rustls::ConnectionCommon<Y>>,
    X: rustls::SideData,
    Y: rustls::SideData,
{
    let mut bytes = Vec::new();
    while from.wants_write() {
        from.write_tls(&mut bytes).unwrap();
    }
    let mut reader = &bytes[..];
    let mut received = Vec::new();
    while !reader.is_empty() {
        to.read_tls(&mut reader).unwrap();
        to.process_new_packets()?;
        // Drained as it comes: rustls buffers at most 16 KiB of it.
        let mut buffer = [0u8; 4096];
        loop {
            match to.reader().read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => received.extend_from_slice(&buffer[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
        }
    }
    Ok(received)
}

/// Handshakes, then sends data both ways. Returns the negotiated suite and group.
fn connect(
    client: Arc<ClientConfig>,
    server: Arc<ServerConfig>,
) -> Result<(CipherSuite, NamedGroup, ProtocolVersion), Error> {
    let mut client =
        ClientConnection::new(client, ServerName::try_from("localhost").unwrap()).unwrap();
    let mut server = ServerConnection::new(server).unwrap();
    for _ in 0..10 {
        if !client.is_handshaking() && !server.is_handshaking() {
            break;
        }
        transfer(&mut client, &mut server)?;
        transfer(&mut server, &mut client)?;
    }
    assert!(!client.is_handshaking() && !server.is_handshaking());
    // Records of several sizes, up to more than one full record, both ways.
    for size in [1usize, 100, 16_384, 40_000] {
        let data: Vec<u8> = (0..size).map(|i| u8::try_from(i % 253).unwrap()).collect();
        client.writer().write_all(&data).unwrap();
        assert_eq!(transfer(&mut client, &mut server)?, data);
        server.writer().write_all(&data).unwrap();
        assert_eq!(transfer(&mut server, &mut client)?, data);
    }
    let suite = client.negotiated_cipher_suite().unwrap().suite();
    let group = client.negotiated_key_exchange_group().unwrap().name();
    let version = client.protocol_version().unwrap();
    Ok((suite, group, version))
}

#[test]
fn handshakes_with_every_suite_group_and_key() {
    const PSS: &[SignatureScheme] = &[SignatureScheme::RSA_PSS_SHA256];
    const PKCS1: &[SignatureScheme] = &[SignatureScheme::RSA_PKCS1_SHA256];
    let mut runs = 0;
    for suite in CIPHER_SUITES {
        for group in KX_GROUPS {
            let tls12 = matches!(suite, SupportedCipherSuite::Tls12(_));
            let ecdsa = suite.suite().as_str().is_some_and(|s| s.contains("ECDSA"));
            // TLS 1.3 suites with either key; TLS 1.2 ones with theirs, and RSA with
            // either signature scheme.
            let mut keys: Vec<(ServerKey, &[u8], &[u8])> = Vec::new();
            if !tls12 || ecdsa {
                keys.push((ec_key(), EE_EC, CA_EC));
            }
            if !tls12 || !ecdsa {
                keys.push((rsa_key_pair(PSS), EE_RSA, CA_RSA));
            }
            if tls12 && !ecdsa {
                keys.push((rsa_key_pair(PKCS1), EE_RSA, CA_RSA));
            }
            for (key, ee, ca) in keys {
                let provider = restricted(*suite, *group);
                let client = client_config(provider.clone(), ca);
                let server = server_config(provider, &[ee], key.clone());
                let (negotiated, kx, version) = connect(client, server)
                    .unwrap_or_else(|e| panic!("{suite:?} {group:?} {key:?}: {e}"));
                assert_eq!(negotiated, suite.suite());
                assert_eq!(kx, group.name());
                let expected = if tls12 {
                    ProtocolVersion::TLSv1_2
                } else {
                    ProtocolVersion::TLSv1_3
                };
                assert_eq!(version, expected);
                runs += 1;
            }
        }
    }
    // 3 TLS 1.3 suites x 2 keys + 3 ECDSA suites + 3 RSA suites x 2 schemes, x 2 groups.
    assert_eq!(runs, 30);
}

#[test]
fn the_default_configuration_negotiates_tls13_with_aes_256() {
    let client = client_config(provider(), CA_EC);
    let server = server_config(provider(), &[EE_EC], ec_key());
    let (suite, group, version) = connect(client, server).unwrap();
    assert_eq!(suite, CipherSuite::TLS13_AES_256_GCM_SHA384);
    assert_eq!(group, NamedGroup::secp256r1);
    assert_eq!(version, ProtocolVersion::TLSv1_3);
}

#[test]
fn refuses_a_server_its_roots_do_not_vouch_for() {
    // The ECDSA server, the RSA root: the chain does not verify.
    let client = client_config(provider(), CA_RSA);
    let server = server_config(provider(), &[EE_EC], ec_key());
    let error = connect(client, server).unwrap_err();
    assert!(matches!(error, Error::InvalidCertificate(_)), "{error:?}");
    // A certificate for another name.
    let mut client = ClientConnection::new(
        client_config(provider(), CA_EC),
        ServerName::try_from("example.org").unwrap(),
    )
    .unwrap();
    let mut server = ServerConnection::new(server_config(provider(), &[EE_EC], ec_key())).unwrap();
    let mut outcome = Ok(Vec::new());
    for _ in 0..4 {
        outcome =
            transfer(&mut client, &mut server).and_then(|_| transfer(&mut server, &mut client));
        if outcome.is_err() {
            break;
        }
    }
    let error = outcome.unwrap_err();
    assert!(matches!(error, Error::InvalidCertificate(_)), "{error:?}");
}

#[test]
fn refuses_a_server_that_signs_with_another_key() {
    // The ECDSA certificate, but the handshake signed with the RSA key: the signature
    // does not verify under the certificate's key.
    let client = client_config(provider(), CA_EC);
    let server = server_config(
        provider(),
        &[EE_EC],
        rsa_key_pair(&[SignatureScheme::RSA_PSS_SHA256]),
    );
    assert!(connect(client, server).is_err());
}

#[test]
fn refuses_a_damaged_record() {
    for suite in CIPHER_SUITES {
        let provider = restricted(*suite, &SECP256R1);
        let ecdsa = suite.suite().as_str().is_some_and(|s| s.contains("ECDSA"));
        let rsa12 = matches!(suite, SupportedCipherSuite::Tls12(_)) && !ecdsa;
        let (key, ee, ca) = if rsa12 {
            (
                rsa_key_pair(&[SignatureScheme::RSA_PSS_SHA256]),
                EE_RSA,
                CA_RSA,
            )
        } else {
            (ec_key(), EE_EC, CA_EC)
        };
        let mut client = ClientConnection::new(
            client_config(provider.clone(), ca),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        let mut server = ServerConnection::new(server_config(provider, &[ee], key)).unwrap();
        while client.is_handshaking() || server.is_handshaking() {
            transfer(&mut client, &mut server).unwrap();
            transfer(&mut server, &mut client).unwrap();
        }
        client.writer().write_all(b"some application data").unwrap();
        let mut bytes = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut bytes).unwrap();
        }
        let last = bytes.len() - 1;
        bytes[last] ^= 0x40;
        let mut reader = &bytes[..];
        server.read_tls(&mut reader).unwrap();
        let error = server.process_new_packets().unwrap_err();
        assert_eq!(error, Error::DecryptError, "{suite:?}");
    }
}

/// On demand (network): a TLS handshake with the provider and the Mozilla roots with
/// each publisher's host, printing what was negotiated. The hosts are those of the
/// pinned downloads, or those of `EINVOICE_TLS_PROBE_HOSTS` (comma-separated).
#[test]
#[ignore = "network: connects to the publishers' hosts"]
fn publishers_hosts_handshake() {
    use std::net::TcpStream;
    let hosts = std::env::var("EINVOICE_TLS_PROBE_HOSTS").unwrap_or_else(|_| {
        "repo1.maven.org,projekte.kosit.org,fnfe-mpe.org,github.com,codeload.github.com".into()
    });
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    for host in hosts.split(',').map(str::trim).filter(|h| !h.is_empty()) {
        let mut tcp = TcpStream::connect((host, 443)).unwrap();
        let mut client = ClientConnection::new(
            Arc::clone(&config),
            ServerName::try_from(host.to_owned()).unwrap(),
        )
        .unwrap();
        client
            .complete_io(&mut tcp)
            .unwrap_or_else(|e| panic!("{host}: {e}"));
        let key = client
            .peer_certificates()
            .and_then(|c| c.first())
            .map_or("none", |c| {
                if c.windows(alg_id::ECDSA_P256.len())
                    .any(|w| w == alg_id::ECDSA_P256.as_ref())
                {
                    "ECDSA P-256"
                } else if c
                    .windows(alg_id::ECDSA_P384.len())
                    .any(|w| w == alg_id::ECDSA_P384.as_ref())
                {
                    "ECDSA P-384"
                } else {
                    "RSA"
                }
            });
        println!(
            "{host}: {:?}, {:?}, {:?}, server key {key}",
            client.protocol_version().unwrap(),
            client.negotiated_cipher_suite().unwrap().suite(),
            client.negotiated_key_exchange_group().unwrap().name(),
        );
    }
}
