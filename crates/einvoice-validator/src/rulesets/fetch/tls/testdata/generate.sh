#!/usr/bin/env bash
# Synthetic test PKI for the TLS provider tests: an ECDSA chain (CA P-384, server P-256)
# and an RSA chain (CA RSA-3072, server RSA-2048), both for the name localhost, valid a
# century; plus signatures of a fixed message for the RSA known-answer tests.
set -euo pipefail
out=$1
work=$(mktemp -d)
cd "$work"
cat > ee.ext <<'EOF'
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost
EOF
cat > ca.ext <<'EOF'
basicConstraints = critical, CA:TRUE
keyUsage = critical, keyCertSign
EOF

# ECDSA.
openssl ecparam -name secp384r1 -genkey -noout -out ca-ec.key
openssl req -new -key ca-ec.key -subj "/CN=einvoice-validator test CA (ECDSA)" -out ca-ec.csr
openssl x509 -req -in ca-ec.csr -signkey ca-ec.key -sha384 -days 36500 -extfile ca.ext -out ca-ec.pem
openssl ecparam -name prime256v1 -genkey -noout -out ee-ec.key
openssl req -new -key ee-ec.key -subj "/CN=localhost" -out ee-ec.csr
openssl x509 -req -in ee-ec.csr -CA ca-ec.pem -CAkey ca-ec.key -CAcreateserial -sha384 -days 36500 -extfile ee.ext -out ee-ec.pem

# RSA.
openssl genrsa -out ca-rsa.key 3072
openssl req -new -key ca-rsa.key -subj "/CN=einvoice-validator test CA (RSA)" -out ca-rsa.csr
openssl x509 -req -in ca-rsa.csr -signkey ca-rsa.key -sha256 -days 36500 -extfile ca.ext -out ca-rsa.pem
openssl genrsa -out ee-rsa.key 2048
openssl req -new -key ee-rsa.key -subj "/CN=localhost" -out ee-rsa.csr
openssl x509 -req -in ee-rsa.csr -CA ca-rsa.pem -CAkey ca-rsa.key -CAcreateserial -sha512 -days 36500 -extfile ee.ext -out ee-rsa.pem

mkdir -p "$out"
for c in ca-ec ee-ec ca-rsa ee-rsa; do
    openssl x509 -in $c.pem -outform DER -out "$out/$c.der"
done
# The P-256 private scalar, raw (32 bytes).
openssl ec -in ee-ec.key -outform DER -out ee-ec.key.der
# SEC 1 ECPrivateKey: SEQUENCE { version, OCTET STRING privateKey, ... }: offset 7, 32 bytes.
[ "$(head -c 7 ee-ec.key.der | od -An -tx1 | tr -d " \n")" = 30770201010420 ] || { echo "unexpected SEC1 layout" >&2; exit 1; }
dd if=ee-ec.key.der of="$out/ee-ec.key" bs=1 skip=7 count=32 status=none
# The RSA private key, PKCS #1 DER.
openssl rsa -in ee-rsa.key -traditional -outform DER -out "$out/ee-rsa.key"

# Known answers: the RSA public key (PKCS #1 DER) and signatures of a message.
printf 'einvoice-validator: a message signed for the known-answer tests\n' > "$out/message.txt"
openssl rsa -in ee-rsa.key -RSAPublicKey_out -outform DER -out "$out/rsa-public.der"
for h in sha256 sha384 sha512; do
    openssl dgst -$h -sign ee-rsa.key -out "$out/rsa-pkcs1-$h.sig" "$out/message.txt"
    openssl dgst -$h -sign ee-rsa.key -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest \
        -out "$out/rsa-pss-$h.sig" "$out/message.txt"
done
openssl ec -in ee-ec.key -pubout -outform DER -out ee-ec-spki.der
# The uncompressed point: the last 65 bytes of the SubjectPublicKeyInfo.
tail -c 65 ee-ec-spki.der > "$out/ec-public.bin"
openssl dgst -sha256 -sign ee-ec.key -out "$out/ecdsa-p256-sha256.sig" "$out/message.txt"
openssl dgst -sha384 -sign ee-ec.key -out "$out/ecdsa-p256-sha384.sig" "$out/message.txt"
openssl verify -CAfile ca-ec.pem ee-ec.pem
openssl verify -CAfile ca-rsa.pem ee-rsa.pem
rm -rf "$work"
