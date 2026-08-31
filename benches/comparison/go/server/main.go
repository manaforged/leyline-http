// Shared HTTPS/2 server for the client comparison. Ephemeral self-signed cert,
// real HTTP/2 over TLS (ALPN h2), keep-alive, fixed 10-byte 200 body. Binds the
// address given (default 127.0.0.1:0 — an ephemeral port, so it never collides
// with anything already running) and prints the actual URL it chose. Every
// client hits this same server, so the only thing that differs is client code.
package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"fmt"
	"math/big"
	"net"
	"net/http"
	"os"
	"time"
)

func selfSigned() tls.Certificate {
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	tmpl := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "localhost"},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(24 * time.Hour),
		DNSNames:              []string{"localhost"},
		IPAddresses:           []net.IP{net.ParseIP("127.0.0.1")},
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}
	der, _ := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	return tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}
}

func main() {
	addr := "127.0.0.1:0"
	if len(os.Args) > 1 {
		addr = os.Args[1]
	}
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		fmt.Fprintln(os.Stderr, "bind error:", err)
		os.Exit(1)
	}
	body := []byte("ok-10byte!")
	mux := http.NewServeMux()
	logProto := os.Getenv("CMP_LOG_PROTO") == "1"
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if logProto {
			fmt.Fprintf(os.Stderr, "PROTO %s\n", r.Proto)
		}
		w.Header().Set("Content-Type", "text/plain")
		w.WriteHeader(200)
		w.Write(body)
	})
	srv := &http.Server{
		Handler: mux,
		TLSConfig: &tls.Config{
			Certificates: []tls.Certificate{selfSigned()},
			NextProtos:   []string{"h2", "http/1.1"},
			MinVersion:   tls.VersionTLS12,
		},
	}
	fmt.Printf("LISTENING https://%s/\n", ln.Addr().String())
	if err := srv.ServeTLS(ln, "", ""); err != nil {
		fmt.Fprintln(os.Stderr, "server error:", err)
		os.Exit(1)
	}
}
