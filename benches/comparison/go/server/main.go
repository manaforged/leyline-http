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
	"strconv"
	"time"
)

func certificate() tls.Certificate {
	if path, ok := os.LookupEnv("CMP_CERT"); ok {
		der, err := os.ReadFile(path)
		if err != nil {
			panic(err)
		}
		keyDer, err := os.ReadFile(os.Getenv("CMP_KEY"))
		if err != nil {
			panic(err)
		}
		key, err := x509.ParsePKCS8PrivateKey(keyDer)
		if err != nil {
			panic(err)
		}
		return tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}
	}
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
	if size, ok := os.LookupEnv("CMP_BODY"); ok {
		n, err := strconv.Atoi(size)
		if err != nil || n < 0 {
			panic("CMP_BODY is a nonnegative byte count")
		}
		body = make([]byte, n)
		for i := range body {
			body[i] = byte(i % 251)
		}
	}
	mux := http.NewServeMux()
	logProto := os.Getenv("CMP_LOG_PROTO") == "1"
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if logProto {
			fmt.Fprintf(os.Stderr, "PROTO %s\n", r.Proto)
			fmt.Fprintf(os.Stderr, "TLS version=%x cipher=%s group=%s resumed=%t retry=%t\n",
				r.TLS.Version, tls.CipherSuiteName(r.TLS.CipherSuite), r.TLS.CurveID, r.TLS.DidResume, r.TLS.HelloRetryRequest)
		}
		w.Header().Set("Content-Type", "text/plain")
		w.WriteHeader(200)
		w.Write(body)
	})
	srv := &http.Server{
		Handler: mux,
		TLSConfig: &tls.Config{
			Certificates: []tls.Certificate{certificate()},
			NextProtos:   []string{"h2", "http/1.1"},
			MinVersion:   tls.VersionTLS12,
		},
	}
	if os.Getenv("CMP_TLS") == "matched" {
		srv.TLSConfig.CurvePreferences = []tls.CurveID{tls.X25519}
	}
	fmt.Printf("LISTENING https://%s/\n", ln.Addr().String())
	if err := srv.ServeTLS(ln, "", ""); err != nil {
		fmt.Fprintln(os.Stderr, "server error:", err)
		os.Exit(1)
	}
}
