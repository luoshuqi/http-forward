#!/usr/bin/env bash

set -e

domain="$1"
if [ -z "$domain" ]; then
  echo "usage: $0 <域名>"
  exit 1
fi

cat << END > v3.ext
[ v3_req ]
basicConstraints = CA:FALSE
keyUsage = nonRepudiation, digitalSignature, keyEncipherment
subjectAltName=DNS:$domain
END

echo "生成服务端证书..."
openssl genrsa -out server_key.pem
openssl req -new -key server_key.pem -out server.csr -subj "/CN=$domain"
openssl x509 -req -days 3650 -signkey server_key.pem -in server.csr -out server_cert.pem -extensions v3_req -extfile v3.ext

echo "生成客户端证书"
sed -i '/^subjectAltName/d' v3.ext
openssl genrsa -out client_key.pem
openssl req -new -key client_key.pem -out client.csr -subj "/CN=client"
openssl x509 -req -days 3650 -CA server_cert.pem -CAkey server_key.pem -CAcreateserial \
  -in client.csr -out client_cert.pem -extensions v3_req -extfile v3.ext
cat server_cert.pem >> client_cert.pem

rm client.csr server.csr v3.ext server_cert.srl

echo "证书已生成"