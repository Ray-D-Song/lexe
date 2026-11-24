#! /bin/bash
set -euo pipefail

cd "$(dirname "$0")"

# Generate the Root CA private key and certificate
openssl req -x509 -newkey rsa:4096 -keyout root.key -out root.pem -days 3650 -nodes -subj "/CN=Test Root CA"

# Default server (localhost)
openssl genrsa -out server.key 2048
openssl req -new -key server.key -out server.csr -subj "/CN=localhost"
cat > extensions.cnf << EOF
subjectAltName = IP:127.0.0.1, DNS:localhost
EOF
openssl x509 -req -in server.csr -CA root.pem -CAkey root.key -CAcreateserial -out server.pem -days 3650 -sha256 -extfile extensions.cnf

# Alternate server for SNI tests (alt.localhost)
openssl genrsa -out alt_server.key 2048
openssl req -new -key alt_server.key -out alt_server.csr -subj "/CN=alt.localhost"
cat > alt_extensions.cnf << EOF
subjectAltName = DNS:alt.localhost
EOF
openssl x509 -req -in alt_server.csr -CA root.pem -CAkey root.key -CAcreateserial -out alt_server.pem -days 3650 -sha256 -extfile alt_extensions.cnf

# Wildcard server for SNI wildcard tests (*.example.com)
openssl genrsa -out wildcard_server.key 2048
openssl req -new -key wildcard_server.key -out wildcard_server.csr -subj "/CN=*.example.com"
cat > wildcard_extensions.cnf << EOF
subjectAltName = DNS:*.example.com
EOF
openssl x509 -req -in wildcard_server.csr -CA root.pem -CAkey root.key -CAcreateserial -out wildcard_server.pem -days 3650 -sha256 -extfile wildcard_extensions.cnf

# mTLS client certificate (CN does not match server hostname)
openssl genrsa -out client.key 2048
openssl req -new -key client.key -out client.csr -subj "/CN=test-client"
cat > client_ext.cnf << EOF
basicConstraints = CA:FALSE
keyUsage = digitalSignature
extendedKeyUsage = clientAuth
EOF
openssl x509 -req -in client.csr -CA root.pem -CAkey root.key -CAcreateserial -out client.pem -days 3650 -sha256 -extfile client_ext.cnf

# Client certificate signed by a different CA (for negative mTLS tests)
openssl req -x509 -newkey rsa:2048 -keyout wrong_root.key -out wrong_root.pem -days 3650 -nodes -subj "/CN=Wrong Root CA"
openssl genrsa -out wrong_client.key 2048
openssl req -new -key wrong_client.key -out wrong_client.csr -subj "/CN=wrong-client"
openssl x509 -req -in wrong_client.csr -CA wrong_root.pem -CAkey wrong_root.key -CAcreateserial -out wrong_client.pem -days 3650 -sha256

# Three-tier chain: root -> intermediate -> leaf
openssl genrsa -out intermediate.key 2048
openssl req -new -key intermediate.key -out intermediate.csr -subj "/CN=Test Intermediate CA"
cat > intermediate_ext.cnf << EOF
[ v3_ca ]
basicConstraints = critical, CA:TRUE, pathlen:0
keyUsage = critical, digitalSignature, keyCertSign, cRLSign
EOF
openssl x509 -req -in intermediate.csr -CA root.pem -CAkey root.key -CAcreateserial -out intermediate.pem -days 3650 -sha256 -extensions v3_ca -extfile intermediate_ext.cnf

openssl genrsa -out chain_leaf.key 2048
openssl req -new -key chain_leaf.key -out chain_leaf.csr -subj "/CN=chain-leaf"
cat > chain_leaf_ext.cnf << EOF
subjectAltName = DNS:chain-leaf.localhost
EOF
openssl x509 -req -in chain_leaf.csr -CA intermediate.pem -CAkey intermediate.key -CAcreateserial -out chain_leaf.pem -days 3650 -sha256 -extfile chain_leaf_ext.cnf

rm -f server.csr alt_server.csr wildcard_server.csr client.csr wrong_client.csr intermediate.csr chain_leaf.csr \
  root.srl wrong_root.srl intermediate.srl chain_leaf.srl \
  extensions.cnf alt_extensions.cnf wildcard_extensions.cnf intermediate_ext.cnf chain_leaf_ext.cnf client_ext.cnf \
  root.key wrong_root.key wrong_root.pem intermediate.key
