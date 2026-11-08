//! The cryptography of the HTTPS downloads: a rustls [`CryptoProvider`] on the `RustCrypto`
//! crates, all pure Rust, for a client only. No C and no assembly file is built; the
//! inline assembly of two of those crates (`cmov`, `zeroize`) is described in
//! `docs/dependencies.md`.
//!
//! rustls runs the protocol (handshakes, key schedules, record layer, certificate path
//! validation with rustls-webpki); this provider only supplies the primitives it asks
//! for, each from a `RustCrypto` crate: SHA-256 and SHA-384 (`sha2`), HMAC (`hmac`), the
//! AEADs AES-GCM (`aes-gcm`) and ChaCha20-Poly1305 (`chacha20poly1305`), ECDHE on P-256
//! and P-384 (`p256`, `p384`), the signature verifications of the Web PKI (ECDSA with
//! `p256`/`p384`, RSA PKCS #1 v1.5 and PSS with `rsa`, public keys only), and the system
//! random source (`getrandom`). Cipher suites: the three of TLS 1.3 and the six ECDHE
//! AEAD suites of TLS 1.2, as rustls' own providers offer them. No private key is ever
//! loaded: the client does not authenticate.
//!
//! What is downloaded is checked against its pinned digest by the caller: TLS is a second
//! line of defence, not the first.

use std::marker::PhantomData;
use std::sync::Arc;

use aes_gcm::aead::{AeadCore, AeadInOut, KeyInit, Nonce as AeadNonce, Tag as AeadTag};
use aes_gcm::{Aes128Gcm, Aes256Gcm};
use chacha20poly1305::ChaCha20Poly1305;
use hmac::Mac as _;
use rustls::crypto::cipher::{
    AeadKey, InboundOpaqueMessage, InboundPlainMessage, Iv, KeyBlockShape, MessageDecrypter,
    MessageEncrypter, NONCE_LEN, Nonce, OutboundOpaqueMessage, OutboundPlainMessage,
    PrefixedPayload, Tls12AeadAlgorithm, Tls13AeadAlgorithm, UnsupportedOperationError,
    make_tls12_aad, make_tls13_aad,
};
use rustls::crypto::hash::HashAlgorithm;
use rustls::crypto::tls12::PrfUsingHmac;
use rustls::crypto::tls13::HkdfUsingHmac;
use rustls::crypto::{
    ActiveKeyExchange, CipherSuiteCommon, CryptoProvider, GetRandomFailed, KeyExchangeAlgorithm,
    KeyProvider, SecureRandom, SharedSecret, SupportedKxGroup, WebPkiSupportedAlgorithms,
};
use rustls::pki_types::alg_id;
use rustls::pki_types::{
    AlgorithmIdentifier, InvalidSignature, PrivateKeyDer, SignatureVerificationAlgorithm,
};
use rustls::sign::SigningKey;
use rustls::{
    CipherSuite, ConnectionTrafficSecrets, ContentType, Error, NamedGroup, ProtocolVersion,
    SignatureScheme, SupportedCipherSuite, Tls12CipherSuite, Tls13CipherSuite,
};
use sha2::{Digest as _, Sha256, Sha384, Sha512};

/// The provider: the cipher suites, key exchange groups and signature verifications
/// below, the system random source, no private keys.
pub(crate) fn provider() -> CryptoProvider {
    CryptoProvider {
        cipher_suites: CIPHER_SUITES.to_vec(),
        kx_groups: KX_GROUPS.to_vec(),
        signature_verification_algorithms: SIGNATURE_ALGORITHMS,
        secure_random: &Provider,
        key_provider: &Provider,
    }
}

/// In rustls' order of preference: TLS 1.3 first, AES-256 before AES-128 before
/// `ChaCha20` (hardware AES where there is some), ECDSA before RSA in TLS 1.2.
static CIPHER_SUITES: &[SupportedCipherSuite] = &[
    SupportedCipherSuite::Tls13(&TLS13_AES_256_GCM_SHA384),
    SupportedCipherSuite::Tls13(&TLS13_AES_128_GCM_SHA256),
    SupportedCipherSuite::Tls13(&TLS13_CHACHA20_POLY1305_SHA256),
    SupportedCipherSuite::Tls12(&TLS12_ECDHE_ECDSA_AES_256_GCM_SHA384),
    SupportedCipherSuite::Tls12(&TLS12_ECDHE_ECDSA_AES_128_GCM_SHA256),
    SupportedCipherSuite::Tls12(&TLS12_ECDHE_ECDSA_CHACHA20_POLY1305_SHA256),
    SupportedCipherSuite::Tls12(&TLS12_ECDHE_RSA_AES_256_GCM_SHA384),
    SupportedCipherSuite::Tls12(&TLS12_ECDHE_RSA_AES_128_GCM_SHA256),
    SupportedCipherSuite::Tls12(&TLS12_ECDHE_RSA_CHACHA20_POLY1305_SHA256),
];

static KX_GROUPS: &[&dyn SupportedKxGroup] = &[&SECP256R1, &SECP384R1];

// ---------------------------------------------------------------------------------------
// Hashes and HMAC.

/// A SHA-2 hash for rustls.
struct Hash<D>(HashAlgorithm, PhantomData<fn() -> D>);

static SHA256: Hash<Sha256> = Hash(HashAlgorithm::SHA256, PhantomData);
static SHA384: Hash<Sha384> = Hash(HashAlgorithm::SHA384, PhantomData);

/// The hashes used here: SHA-256 and SHA-384 (`Clone` to fork a running hash).
trait Sha2: sha2::Digest + Clone + Send + Sync + 'static {}
impl Sha2 for Sha256 {}
impl Sha2 for Sha384 {}

impl<D: Sha2> rustls::crypto::hash::Hash for Hash<D> {
    fn start(&self) -> Box<dyn rustls::crypto::hash::Context> {
        Box::new(HashContext(D::new()))
    }

    fn hash(&self, data: &[u8]) -> rustls::crypto::hash::Output {
        rustls::crypto::hash::Output::new(&D::digest(data))
    }

    fn output_len(&self) -> usize {
        <D as sha2::Digest>::output_size()
    }

    fn algorithm(&self) -> HashAlgorithm {
        self.0
    }
}

struct HashContext<D>(D);

impl<D: Sha2> rustls::crypto::hash::Context for HashContext<D> {
    fn fork_finish(&self) -> rustls::crypto::hash::Output {
        rustls::crypto::hash::Output::new(&self.0.clone().finalize())
    }

    fn fork(&self) -> Box<dyn rustls::crypto::hash::Context> {
        Box::new(Self(self.0.clone()))
    }

    fn finish(self: Box<Self>) -> rustls::crypto::hash::Output {
        rustls::crypto::hash::Output::new(&self.0.finalize())
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }
}

/// HMAC on a SHA-2 hash, for rustls (the TLS 1.3 HKDF and the TLS 1.2 PRF are built on
/// it by rustls).
struct Hmac<D>(PhantomData<fn() -> D>);

static HMAC_SHA256: Hmac<Sha256> = Hmac(PhantomData);
static HMAC_SHA384: Hmac<Sha384> = Hmac(PhantomData);

impl<D> rustls::crypto::hmac::Hmac for Hmac<D>
where
    D: Sha2 + hmac::EagerHash,
    hmac::Hmac<D>: Send + Sync,
{
    fn with_key(&self, key: &[u8]) -> Box<dyn rustls::crypto::hmac::Key> {
        #[expect(
            clippy::expect_used,
            reason = "HMAC takes a key of any length: new_from_slice never fails"
        )]
        let mac = <hmac::Hmac<D> as hmac::KeyInit>::new_from_slice(key)
            .expect("HMAC accepts keys of any length");
        Box::new(HmacKey(mac))
    }

    fn hash_output_len(&self) -> usize {
        <D as sha2::Digest>::output_size()
    }
}

struct HmacKey<D: hmac::EagerHash>(hmac::Hmac<D>);

impl<D> rustls::crypto::hmac::Key for HmacKey<D>
where
    D: Sha2 + hmac::EagerHash,
    hmac::Hmac<D>: Send + Sync,
{
    fn sign_concat(
        &self,
        first: &[u8],
        middle: &[&[u8]],
        last: &[u8],
    ) -> rustls::crypto::hmac::Tag {
        let mut mac = self.0.clone();
        mac.update(first);
        for part in middle {
            mac.update(part);
        }
        mac.update(last);
        rustls::crypto::hmac::Tag::new(&mac.finalize().into_bytes())
    }

    fn tag_len(&self) -> usize {
        <D as sha2::Digest>::output_size()
    }
}

// ---------------------------------------------------------------------------------------
// TLS 1.3 cipher suites.

static TLS13_AES_256_GCM_SHA384: Tls13CipherSuite = Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_AES_256_GCM_SHA384,
        hash_provider: &SHA384,
        confidentiality_limit: 1 << 24,
    },
    hkdf_provider: &HkdfUsingHmac(&HMAC_SHA384),
    aead_alg: &Tls13Aead::<Aes256Gcm> {
        aead: Aead::Aes256Gcm,
        cipher: PhantomData,
    },
    quic: None,
};

static TLS13_AES_128_GCM_SHA256: Tls13CipherSuite = Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_AES_128_GCM_SHA256,
        hash_provider: &SHA256,
        confidentiality_limit: 1 << 24,
    },
    hkdf_provider: &HkdfUsingHmac(&HMAC_SHA256),
    aead_alg: &Tls13Aead::<Aes128Gcm> {
        aead: Aead::Aes128Gcm,
        cipher: PhantomData,
    },
    quic: None,
};

static TLS13_CHACHA20_POLY1305_SHA256: Tls13CipherSuite = Tls13CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
        hash_provider: &SHA256,
        confidentiality_limit: u64::MAX,
    },
    hkdf_provider: &HkdfUsingHmac(&HMAC_SHA256),
    aead_alg: &Tls13Aead::<ChaCha20Poly1305> {
        aead: Aead::ChaCha20Poly1305,
        cipher: PhantomData,
    },
    quic: None,
};

/// The AEADs of the suites, and what rustls needs to know of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aead {
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl Aead {
    const fn key_len(self) -> usize {
        match self {
            Self::Aes128Gcm => 16,
            Self::Aes256Gcm | Self::ChaCha20Poly1305 => 32,
        }
    }

    fn secrets(self, key: AeadKey, iv: Iv) -> ConnectionTrafficSecrets {
        match self {
            Self::Aes128Gcm => ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            Self::Aes256Gcm => ConnectionTrafficSecrets::Aes256Gcm { key, iv },
            Self::ChaCha20Poly1305 => ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv },
        }
    }
}

/// The length of the authentication tag of every AEAD here.
const TAG_LEN: usize = 16;
/// The largest plaintext of a TLS record (2^14 bytes).
const MAX_FRAGMENT_LEN: usize = 16_384;

/// An AEAD cipher of the `RustCrypto` crates, keyed for one direction of a connection.
/// `None` when rustls gave a key of the wrong length, which it never does: every record
/// is then refused.
trait Cipher: AeadInOut + KeyInit + Send + Sync + 'static {}
impl Cipher for Aes128Gcm {}
impl Cipher for Aes256Gcm {}
impl Cipher for ChaCha20Poly1305 {}

fn keyed<C: Cipher>(key: &AeadKey) -> Option<C> {
    C::new_from_slice(key.as_ref()).ok()
}

/// Encrypts `buffer` in place with `nonce` and `aad`, returning the tag.
fn seal<C: Cipher>(
    cipher: Option<&C>,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    buffer: &mut [u8],
) -> Result<AeadTag<C>, Error> {
    let cipher = cipher.ok_or(Error::EncryptError)?;
    let nonce = AeadNonce::<C>::try_from(&nonce[..]).map_err(|_| Error::EncryptError)?;
    cipher
        .encrypt_inout_detached(&nonce, aad, buffer.into())
        .map_err(|_| Error::EncryptError)
}

/// Decrypts `buffer` (the ciphertext, then the tag) in place with `nonce` and `aad`;
/// returns the length of the plaintext, at the start of `buffer`.
fn open<C: Cipher>(
    cipher: Option<&C>,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    buffer: &mut [u8],
) -> Result<usize, Error> {
    let cipher = cipher.ok_or(Error::DecryptError)?;
    let plain_len = buffer
        .len()
        .checked_sub(TAG_LEN)
        .ok_or(Error::DecryptError)?;
    let nonce = AeadNonce::<C>::try_from(&nonce[..]).map_err(|_| Error::DecryptError)?;
    let (text, tag) = buffer.split_at_mut(plain_len);
    let tag = AeadTag::<C>::try_from(&*tag).map_err(|_| Error::DecryptError)?;
    cipher
        .decrypt_inout_detached(&nonce, aad, text.into(), &tag)
        .map_err(|_| Error::DecryptError)?;
    Ok(plain_len)
}

/// The TLS 1.3 record protection with the AEAD `C`.
struct Tls13Aead<C> {
    aead: Aead,
    cipher: PhantomData<fn() -> C>,
}

impl<C: Cipher> Tls13AeadAlgorithm for Tls13Aead<C>
where
    C: AeadCore,
{
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> {
        Box::new(Tls13Encrypter::<C> {
            cipher: keyed(&key),
            iv,
        })
    }

    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> {
        Box::new(Tls13Decrypter::<C> {
            cipher: keyed(&key),
            iv,
        })
    }

    fn key_len(&self) -> usize {
        self.aead.key_len()
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: Iv,
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(self.aead.secrets(key, iv))
    }
}

struct Tls13Encrypter<C> {
    cipher: Option<C>,
    iv: Iv,
}

impl<C: Cipher> MessageEncrypter for Tls13Encrypter<C> {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, Error> {
        let total_len = self.encrypted_payload_len(msg.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total_len);
        payload.extend_from_chunks(&msg.payload);
        payload.extend_from_slice(&msg.typ.to_array());
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls13_aad(total_len);
        let tag = seal(self.cipher.as_ref(), &nonce, &aad, payload.as_mut())?;
        payload.extend_from_slice(&tag);
        // Every TLS 1.3 record says TLS 1.2 (RFC 8446, 5.1).
        Ok(OutboundOpaqueMessage::new(
            ContentType::ApplicationData,
            ProtocolVersion::TLSv1_2,
            payload,
        ))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + 1 + TAG_LEN
    }
}

struct Tls13Decrypter<C> {
    cipher: Option<C>,
    iv: Iv,
}

impl<C: Cipher> MessageDecrypter for Tls13Decrypter<C> {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, Error> {
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls13_aad(msg.payload.len());
        let plain_len = open(self.cipher.as_ref(), &nonce, &aad, &mut msg.payload)?;
        msg.payload.truncate(plain_len);
        msg.into_tls13_unpadded_message()
    }
}

// ---------------------------------------------------------------------------------------
// TLS 1.2 cipher suites.

static TLS12_ECDHE_ECDSA_AES_256_GCM_SHA384: Tls12CipherSuite = Tls12CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        hash_provider: &SHA384,
        confidentiality_limit: 1 << 24,
    },
    kx: KeyExchangeAlgorithm::ECDHE,
    sign: TLS12_ECDSA_SCHEMES,
    aead_alg: &Tls12Gcm::<Aes256Gcm> {
        aead: Aead::Aes256Gcm,
        cipher: PhantomData,
    },
    prf_provider: &PrfUsingHmac(&HMAC_SHA384),
};

static TLS12_ECDHE_ECDSA_AES_128_GCM_SHA256: Tls12CipherSuite = Tls12CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        hash_provider: &SHA256,
        confidentiality_limit: 1 << 24,
    },
    kx: KeyExchangeAlgorithm::ECDHE,
    sign: TLS12_ECDSA_SCHEMES,
    aead_alg: &Tls12Gcm::<Aes128Gcm> {
        aead: Aead::Aes128Gcm,
        cipher: PhantomData,
    },
    prf_provider: &PrfUsingHmac(&HMAC_SHA256),
};

static TLS12_ECDHE_ECDSA_CHACHA20_POLY1305_SHA256: Tls12CipherSuite = Tls12CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
        hash_provider: &SHA256,
        confidentiality_limit: u64::MAX,
    },
    kx: KeyExchangeAlgorithm::ECDHE,
    sign: TLS12_ECDSA_SCHEMES,
    aead_alg: &Tls12ChaCha,
    prf_provider: &PrfUsingHmac(&HMAC_SHA256),
};

static TLS12_ECDHE_RSA_AES_256_GCM_SHA384: Tls12CipherSuite = Tls12CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        hash_provider: &SHA384,
        confidentiality_limit: 1 << 24,
    },
    kx: KeyExchangeAlgorithm::ECDHE,
    sign: TLS12_RSA_SCHEMES,
    aead_alg: &Tls12Gcm::<Aes256Gcm> {
        aead: Aead::Aes256Gcm,
        cipher: PhantomData,
    },
    prf_provider: &PrfUsingHmac(&HMAC_SHA384),
};

static TLS12_ECDHE_RSA_AES_128_GCM_SHA256: Tls12CipherSuite = Tls12CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        hash_provider: &SHA256,
        confidentiality_limit: 1 << 24,
    },
    kx: KeyExchangeAlgorithm::ECDHE,
    sign: TLS12_RSA_SCHEMES,
    aead_alg: &Tls12Gcm::<Aes128Gcm> {
        aead: Aead::Aes128Gcm,
        cipher: PhantomData,
    },
    prf_provider: &PrfUsingHmac(&HMAC_SHA256),
};

static TLS12_ECDHE_RSA_CHACHA20_POLY1305_SHA256: Tls12CipherSuite = Tls12CipherSuite {
    common: CipherSuiteCommon {
        suite: CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
        hash_provider: &SHA256,
        confidentiality_limit: u64::MAX,
    },
    kx: KeyExchangeAlgorithm::ECDHE,
    sign: TLS12_RSA_SCHEMES,
    aead_alg: &Tls12ChaCha,
    prf_provider: &PrfUsingHmac(&HMAC_SHA256),
};

/// The signature schemes of the TLS 1.2 ECDSA suites (those verified here).
static TLS12_ECDSA_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::ECDSA_NISTP384_SHA384,
    SignatureScheme::ECDSA_NISTP256_SHA256,
];

/// The signature schemes of the TLS 1.2 RSA suites.
static TLS12_RSA_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::RSA_PSS_SHA512,
    SignatureScheme::RSA_PSS_SHA384,
    SignatureScheme::RSA_PSS_SHA256,
    SignatureScheme::RSA_PKCS1_SHA512,
    SignatureScheme::RSA_PKCS1_SHA384,
    SignatureScheme::RSA_PKCS1_SHA256,
];

/// The length of the explicit part of a TLS 1.2 AES-GCM nonce, sent in each record
/// (RFC 5288).
const GCM_EXPLICIT_NONCE_LEN: usize = 8;
/// The implicit part (the salt), from the key block.
const GCM_SALT_LEN: usize = 4;

/// The TLS 1.2 AES-GCM record protection.
struct Tls12Gcm<C> {
    aead: Aead,
    cipher: PhantomData<fn() -> C>,
}

/// The nonce of a TLS 1.2 AES-GCM record: the salt, then the explicit part.
fn gcm_iv(salt: &[u8], explicit: &[u8]) -> Iv {
    let mut iv = [0u8; NONCE_LEN];
    if salt.len() == GCM_SALT_LEN && explicit.len() == GCM_EXPLICIT_NONCE_LEN {
        iv[..GCM_SALT_LEN].copy_from_slice(salt);
        iv[GCM_SALT_LEN..].copy_from_slice(explicit);
    }
    Iv::new(iv)
}

impl<C: Cipher> Tls12AeadAlgorithm for Tls12Gcm<C> {
    fn encrypter(&self, key: AeadKey, iv: &[u8], extra: &[u8]) -> Box<dyn MessageEncrypter> {
        Box::new(Tls12GcmEncrypter::<C> {
            cipher: keyed(&key),
            iv: gcm_iv(iv, extra),
        })
    }

    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        let mut salt = [0u8; GCM_SALT_LEN];
        let cipher = if iv.len() == GCM_SALT_LEN {
            salt.copy_from_slice(iv);
            keyed(&key)
        } else {
            None
        };
        Box::new(Tls12GcmDecrypter::<C> { cipher, salt })
    }

    fn key_block_shape(&self) -> KeyBlockShape {
        KeyBlockShape {
            enc_key_len: self.aead.key_len(),
            fixed_iv_len: GCM_SALT_LEN,
            explicit_nonce_len: GCM_EXPLICIT_NONCE_LEN,
        }
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: &[u8],
        explicit: &[u8],
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(self.aead.secrets(key, gcm_iv(iv, explicit)))
    }
}

struct Tls12GcmEncrypter<C> {
    cipher: Option<C>,
    iv: Iv,
}

impl<C: Cipher> MessageEncrypter for Tls12GcmEncrypter<C> {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, Error> {
        let total_len = self.encrypted_payload_len(msg.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total_len);
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls12_aad(seq, msg.typ, msg.version, msg.payload.len());
        payload.extend_from_slice(&nonce[GCM_SALT_LEN..]);
        payload.extend_from_chunks(&msg.payload);
        let tag = seal(
            self.cipher.as_ref(),
            &nonce,
            &aad,
            &mut payload.as_mut()[GCM_EXPLICIT_NONCE_LEN..],
        )?;
        payload.extend_from_slice(&tag);
        Ok(OutboundOpaqueMessage::new(msg.typ, msg.version, payload))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + GCM_EXPLICIT_NONCE_LEN + TAG_LEN
    }
}

struct Tls12GcmDecrypter<C> {
    cipher: Option<C>,
    salt: [u8; GCM_SALT_LEN],
}

impl<C: Cipher> MessageDecrypter for Tls12GcmDecrypter<C> {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, Error> {
        let payload = &mut msg.payload;
        if payload.len() < GCM_EXPLICIT_NONCE_LEN + TAG_LEN {
            return Err(Error::DecryptError);
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce[..GCM_SALT_LEN].copy_from_slice(&self.salt);
        nonce[GCM_SALT_LEN..].copy_from_slice(&payload[..GCM_EXPLICIT_NONCE_LEN]);
        let aad = make_tls12_aad(
            seq,
            msg.typ,
            msg.version,
            payload.len() - GCM_EXPLICIT_NONCE_LEN - TAG_LEN,
        );
        let plain_len = open(
            self.cipher.as_ref(),
            &nonce,
            &aad,
            &mut payload[GCM_EXPLICIT_NONCE_LEN..],
        )?;
        if plain_len > MAX_FRAGMENT_LEN {
            return Err(Error::PeerSentOversizedRecord);
        }
        Ok(
            msg.into_plain_message_range(
                GCM_EXPLICIT_NONCE_LEN..GCM_EXPLICIT_NONCE_LEN + plain_len,
            ),
        )
    }
}

/// The TLS 1.2 ChaCha20-Poly1305 record protection (RFC 7905): the nonce is the IV
/// XOR the sequence number, as in TLS 1.3.
struct Tls12ChaCha;

impl Tls12AeadAlgorithm for Tls12ChaCha {
    fn encrypter(&self, key: AeadKey, iv: &[u8], _: &[u8]) -> Box<dyn MessageEncrypter> {
        Box::new(Tls12ChaChaEncrypter {
            cipher: if iv.len() == NONCE_LEN {
                keyed(&key)
            } else {
                None
            },
            iv: Iv::copy(iv),
        })
    }

    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        Box::new(Tls12ChaChaDecrypter {
            cipher: if iv.len() == NONCE_LEN {
                keyed(&key)
            } else {
                None
            },
            iv: Iv::copy(iv),
        })
    }

    fn key_block_shape(&self) -> KeyBlockShape {
        KeyBlockShape {
            enc_key_len: Aead::ChaCha20Poly1305.key_len(),
            fixed_iv_len: NONCE_LEN,
            explicit_nonce_len: 0,
        }
    }

    fn extract_keys(
        &self,
        key: AeadKey,
        iv: &[u8],
        _explicit: &[u8],
    ) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        if iv.len() != NONCE_LEN {
            return Err(UnsupportedOperationError);
        }
        Ok(Aead::ChaCha20Poly1305.secrets(key, Iv::copy(iv)))
    }
}

struct Tls12ChaChaEncrypter {
    cipher: Option<ChaCha20Poly1305>,
    iv: Iv,
}

impl MessageEncrypter for Tls12ChaChaEncrypter {
    fn encrypt(
        &mut self,
        msg: OutboundPlainMessage<'_>,
        seq: u64,
    ) -> Result<OutboundOpaqueMessage, Error> {
        let total_len = self.encrypted_payload_len(msg.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total_len);
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls12_aad(seq, msg.typ, msg.version, msg.payload.len());
        payload.extend_from_chunks(&msg.payload);
        let tag = seal(self.cipher.as_ref(), &nonce, &aad, payload.as_mut())?;
        payload.extend_from_slice(&tag);
        Ok(OutboundOpaqueMessage::new(msg.typ, msg.version, payload))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + TAG_LEN
    }
}

struct Tls12ChaChaDecrypter {
    cipher: Option<ChaCha20Poly1305>,
    iv: Iv,
}

impl MessageDecrypter for Tls12ChaChaDecrypter {
    fn decrypt<'a>(
        &mut self,
        mut msg: InboundOpaqueMessage<'a>,
        seq: u64,
    ) -> Result<InboundPlainMessage<'a>, Error> {
        let payload = &mut msg.payload;
        if payload.len() < TAG_LEN {
            return Err(Error::DecryptError);
        }
        let nonce = Nonce::new(&self.iv, seq).0;
        let aad = make_tls12_aad(seq, msg.typ, msg.version, payload.len() - TAG_LEN);
        let plain_len = open(self.cipher.as_ref(), &nonce, &aad, payload)?;
        if plain_len > MAX_FRAGMENT_LEN {
            return Err(Error::PeerSentOversizedRecord);
        }
        payload.truncate(plain_len);
        Ok(msg.into_plain_message())
    }
}

// ---------------------------------------------------------------------------------------
// Key exchange: ECDHE on P-256 and P-384.

/// An ECDHE group on the curve `C`.
struct EcGroup<C>(NamedGroup, PhantomData<fn() -> C>);

static SECP256R1: EcGroup<p256::NistP256> = EcGroup(NamedGroup::secp256r1, PhantomData);
static SECP384R1: EcGroup<p384::NistP384> = EcGroup(NamedGroup::secp384r1, PhantomData);

impl<C> std::fmt::Debug for EcGroup<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EcGroup").field(&self.0).finish()
    }
}

/// A private key and its public key (SEC 1, uncompressed).
type KeyPair = (Box<[u8]>, Box<[u8]>);

/// The curves used here.
trait Curve: Send + Sync + 'static {
    /// The length of a field element, in bytes.
    const FIELD_LEN: usize;
    /// A fresh private key and its public key (SEC 1, uncompressed).
    fn generate() -> Result<KeyPair, Error>;
    /// The shared secret of the private key `secret` and the peer's public key `peer`
    /// (SEC 1, uncompressed only, as TLS requires).
    fn agree(secret: &[u8], peer: &[u8]) -> Result<Vec<u8>, Error>;
}

macro_rules! curve {
    ($curve:ty, $krate:ident, $field_len:literal) => {
        impl Curve for $curve {
            const FIELD_LEN: usize = $field_len;

            fn generate() -> Result<KeyPair, Error> {
                use $krate::elliptic_curve::sec1::ToSec1Point as _;
                // A random scalar in [1, n - 1]: a draw out of range (probability below
                // 2^-32) is drawn again.
                for _ in 0..64 {
                    let mut bytes = $krate::FieldBytes::default();
                    getrandom::fill(&mut bytes).map_err(|_| Error::FailedToGetRandomBytes)?;
                    if let Ok(secret) = $krate::SecretKey::from_bytes(&bytes) {
                        bytes.fill(0);
                        let public = secret.public_key().to_sec1_point(false);
                        return Ok((
                            Box::from(&secret.to_bytes()[..]),
                            Box::from(public.as_bytes()),
                        ));
                    }
                }
                Err(Error::FailedToGetRandomBytes)
            }

            fn agree(secret: &[u8], peer: &[u8]) -> Result<Vec<u8>, Error> {
                let invalid = || Error::PeerMisbehaved(rustls::PeerMisbehaved::InvalidKeyShare);
                if peer.len() != 1 + 2 * Self::FIELD_LEN || peer.first() != Some(&4) {
                    return Err(invalid());
                }
                let peer = $krate::PublicKey::from_sec1_bytes(peer).map_err(|_| invalid())?;
                let secret = $krate::SecretKey::from_slice(secret)
                    .map_err(|_| Error::General("bad ECDHE private key".into()))?;
                Ok(secret.diffie_hellman(&peer).raw_secret_bytes().to_vec())
            }
        }
    };
}

curve!(p256::NistP256, p256, 32);
curve!(p384::NistP384, p384, 48);

impl<C: Curve> SupportedKxGroup for EcGroup<C> {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let (secret, public) = C::generate()?;
        Ok(Box::new(EcExchange::<C> {
            group: self.0,
            secret,
            public,
            curve: PhantomData,
        }))
    }

    fn name(&self) -> NamedGroup {
        self.0
    }
}

/// A key exchange under way: our private and public keys.
struct EcExchange<C> {
    group: NamedGroup,
    secret: Box<[u8]>,
    public: Box<[u8]>,
    curve: PhantomData<fn() -> C>,
}

impl<C> Drop for EcExchange<C> {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

impl<C: Curve> ActiveKeyExchange for EcExchange<C> {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        let mut shared = C::agree(&self.secret, peer_pub_key)?;
        let secret = SharedSecret::from(&shared[..]);
        shared.fill(0);
        Ok(secret)
    }

    fn pub_key(&self) -> &[u8] {
        &self.public
    }

    fn group(&self) -> NamedGroup {
        self.group
    }
}

// ---------------------------------------------------------------------------------------
// Signature verification (certificates and handshakes).

/// A signature verification algorithm of the Web PKI.
#[derive(Debug)]
struct Verify {
    scheme: Scheme,
    public_key: AlgorithmIdentifier,
    signature: AlgorithmIdentifier,
}

#[derive(Debug, Clone, Copy)]
enum HashKind {
    Sha256,
    Sha384,
    Sha512,
}

#[derive(Debug, Clone, Copy)]
enum Scheme {
    /// ECDSA on P-256 (ASN.1 DER signatures).
    EcdsaP256(HashKind),
    /// ECDSA on P-384.
    EcdsaP384(HashKind),
    /// RSA PKCS #1 v1.5, keys of 2048 to 8192 bits.
    RsaPkcs1(HashKind),
    /// RSA PSS, salt as long as the hash (RFC 8446, 4.2.3), keys of 2048 to 8192 bits.
    RsaPss(HashKind),
}

/// The smallest RSA key accepted, in bits (as rustls' own providers).
const MIN_RSA_BITS: u32 = 2048;

impl HashKind {
    fn digest(self, message: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Sha256::digest(message).to_vec(),
            Self::Sha384 => Sha384::digest(message).to_vec(),
            Self::Sha512 => Sha512::digest(message).to_vec(),
        }
    }
}

fn rsa_key(public_key: &[u8]) -> Result<rsa::PublicKey, InvalidSignature> {
    use rsa::pkcs1::DecodeRsaPublicKey as _;
    use rsa::traits::PublicKeyParts as _;
    let key = rsa::PublicKey::from_pkcs1_der(public_key).map_err(|_| InvalidSignature)?;
    if key.n().bits_vartime() < MIN_RSA_BITS {
        return Err(InvalidSignature);
    }
    Ok(key)
}

fn verify_pkcs1(
    hash: HashKind,
    key: rsa::PublicKey,
    message: &[u8],
    signature: &[u8],
) -> Result<(), InvalidSignature> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier as _;
    let signature = Signature::try_from(signature).map_err(|_| InvalidSignature)?;
    match hash {
        HashKind::Sha256 => VerifyingKey::<Sha256>::new(key).verify(message, &signature),
        HashKind::Sha384 => VerifyingKey::<Sha384>::new(key).verify(message, &signature),
        HashKind::Sha512 => VerifyingKey::<Sha512>::new(key).verify(message, &signature),
    }
    .map_err(|_| InvalidSignature)
}

fn verify_pss(
    hash: HashKind,
    key: rsa::PublicKey,
    message: &[u8],
    signature: &[u8],
) -> Result<(), InvalidSignature> {
    use rsa::pss::{Signature, VerifyingKey};
    use rsa::signature::Verifier as _;
    let signature = Signature::try_from(signature).map_err(|_| InvalidSignature)?;
    // `new`: the salt is as long as the hash.
    match hash {
        HashKind::Sha256 => VerifyingKey::<Sha256>::new(key).verify(message, &signature),
        HashKind::Sha384 => VerifyingKey::<Sha384>::new(key).verify(message, &signature),
        HashKind::Sha512 => VerifyingKey::<Sha512>::new(key).verify(message, &signature),
    }
    .map_err(|_| InvalidSignature)
}

impl SignatureVerificationAlgorithm for Verify {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        use p256::ecdsa::signature::hazmat::PrehashVerifier as _;
        match self.scheme {
            Scheme::EcdsaP256(hash) => {
                let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key)
                    .map_err(|_| InvalidSignature)?;
                let signature =
                    p256::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
                key.verify_prehash(&hash.digest(message), &signature)
                    .map_err(|_| InvalidSignature)
            }
            Scheme::EcdsaP384(hash) => {
                let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(public_key)
                    .map_err(|_| InvalidSignature)?;
                let signature =
                    p384::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
                key.verify_prehash(&hash.digest(message), &signature)
                    .map_err(|_| InvalidSignature)
            }
            Scheme::RsaPkcs1(hash) => verify_pkcs1(hash, rsa_key(public_key)?, message, signature),
            Scheme::RsaPss(hash) => verify_pss(hash, rsa_key(public_key)?, message, signature),
        }
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        self.public_key
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        self.signature
    }
}

static ECDSA_P256_SHA256: Verify = Verify {
    scheme: Scheme::EcdsaP256(HashKind::Sha256),
    public_key: alg_id::ECDSA_P256,
    signature: alg_id::ECDSA_SHA256,
};
static ECDSA_P256_SHA384: Verify = Verify {
    scheme: Scheme::EcdsaP256(HashKind::Sha384),
    public_key: alg_id::ECDSA_P256,
    signature: alg_id::ECDSA_SHA384,
};
static ECDSA_P384_SHA256: Verify = Verify {
    scheme: Scheme::EcdsaP384(HashKind::Sha256),
    public_key: alg_id::ECDSA_P384,
    signature: alg_id::ECDSA_SHA256,
};
static ECDSA_P384_SHA384: Verify = Verify {
    scheme: Scheme::EcdsaP384(HashKind::Sha384),
    public_key: alg_id::ECDSA_P384,
    signature: alg_id::ECDSA_SHA384,
};
static RSA_PKCS1_SHA256: Verify = Verify {
    scheme: Scheme::RsaPkcs1(HashKind::Sha256),
    public_key: alg_id::RSA_ENCRYPTION,
    signature: alg_id::RSA_PKCS1_SHA256,
};
static RSA_PKCS1_SHA384: Verify = Verify {
    scheme: Scheme::RsaPkcs1(HashKind::Sha384),
    public_key: alg_id::RSA_ENCRYPTION,
    signature: alg_id::RSA_PKCS1_SHA384,
};
static RSA_PKCS1_SHA512: Verify = Verify {
    scheme: Scheme::RsaPkcs1(HashKind::Sha512),
    public_key: alg_id::RSA_ENCRYPTION,
    signature: alg_id::RSA_PKCS1_SHA512,
};
static RSA_PSS_SHA256: Verify = Verify {
    scheme: Scheme::RsaPss(HashKind::Sha256),
    public_key: alg_id::RSA_ENCRYPTION,
    signature: alg_id::RSA_PSS_SHA256,
};
static RSA_PSS_SHA384: Verify = Verify {
    scheme: Scheme::RsaPss(HashKind::Sha384),
    public_key: alg_id::RSA_ENCRYPTION,
    signature: alg_id::RSA_PSS_SHA384,
};
static RSA_PSS_SHA512: Verify = Verify {
    scheme: Scheme::RsaPss(HashKind::Sha512),
    public_key: alg_id::RSA_ENCRYPTION,
    signature: alg_id::RSA_PSS_SHA512,
};

/// The verifications, for certificates (`all`) and for the handshake signatures of each
/// TLS signature scheme (`mapping`), as rustls' own providers map them (without Ed25519,
/// which no public root or publisher here uses).
static SIGNATURE_ALGORITHMS: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: &[
        &ECDSA_P256_SHA256,
        &ECDSA_P256_SHA384,
        &ECDSA_P384_SHA256,
        &ECDSA_P384_SHA384,
        &RSA_PSS_SHA256,
        &RSA_PSS_SHA384,
        &RSA_PSS_SHA512,
        &RSA_PKCS1_SHA256,
        &RSA_PKCS1_SHA384,
        &RSA_PKCS1_SHA512,
    ],
    mapping: &[
        // In TLS 1.2 the curve is not fixed by the scheme; in TLS 1.3 it is.
        (
            SignatureScheme::ECDSA_NISTP384_SHA384,
            &[&ECDSA_P384_SHA384, &ECDSA_P256_SHA384],
        ),
        (
            SignatureScheme::ECDSA_NISTP256_SHA256,
            &[&ECDSA_P256_SHA256, &ECDSA_P384_SHA256],
        ),
        (SignatureScheme::RSA_PSS_SHA512, &[&RSA_PSS_SHA512]),
        (SignatureScheme::RSA_PSS_SHA384, &[&RSA_PSS_SHA384]),
        (SignatureScheme::RSA_PSS_SHA256, &[&RSA_PSS_SHA256]),
        (SignatureScheme::RSA_PKCS1_SHA512, &[&RSA_PKCS1_SHA512]),
        (SignatureScheme::RSA_PKCS1_SHA384, &[&RSA_PKCS1_SHA384]),
        (SignatureScheme::RSA_PKCS1_SHA256, &[&RSA_PKCS1_SHA256]),
    ],
};

// ---------------------------------------------------------------------------------------
// Random numbers and (no) private keys.

#[derive(Debug)]
struct Provider;

impl SecureRandom for Provider {
    fn fill(&self, buf: &mut [u8]) -> Result<(), GetRandomFailed> {
        getrandom::fill(buf).map_err(|_| GetRandomFailed)
    }
}

impl KeyProvider for Provider {
    fn load_private_key(
        &self,
        _key_der: PrivateKeyDer<'static>,
    ) -> Result<Arc<dyn SigningKey>, Error> {
        Err(Error::General(
            "no private key: the downloads do not authenticate the client".into(),
        ))
    }
}

#[cfg(test)]
mod tests;
