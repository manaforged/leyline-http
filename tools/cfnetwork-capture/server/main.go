// cfnetwork-capture server: TLS-terminating HTTP/2 capture target.
//
// Reads every client frame with x/net/http2.Framer and logs frame ORDER and
// payload — client SETTINGS (incl. per-setting order), WINDOW_UPDATE,
// HEADERS pseudo-header order (HPACK-decoded), PING, RST_STREAM, GOAWAY.
// That ordering is the H2 half of the CFNetwork wire fingerprint.
//
// The TLS half (ClientHello) is captured separately with tcpdump by
// capture.sh; this server terminates TLS so the H2 exchange completes.
//
// Base: the self-signed cert pattern from benches/comparison/go/server.
package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/pem"
	"fmt"
	"math/big"
	"net"
	"os"
	"time"

	"golang.org/x/net/http2"
	"golang.org/x/net/http2/hpack"
)

func selfSigned() tls.Certificate {
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	tmpl := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject:      pkix.Name{CommonName: "cfnetwork-capture"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(24 * time.Hour),
		DNSNames:     []string{"localhost"},
		IPAddresses:  []net.IP{net.ParseIP("127.0.0.1")},
		KeyUsage:     x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}
	der, _ := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	cert := tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}
	// Persist the cert (and key) so capture.sh / operators can install it
	// where a probe delegate isn't used. Ephemeral otherwise.
	if out := os.Getenv("CAPTURE_CERT_OUT"); out != "" {
		cf, _ := os.Create(out + ".crt")
		pem.Encode(cf, &pem.Block{Type: "CERTIFICATE", Bytes: der})
		cf.Close()
		derKey, _ := x509.MarshalECPrivateKey(key)
		kf, _ := os.Create(out + ".key")
		pem.Encode(kf, &pem.Block{Type: "EC PRIVATE KEY", Bytes: derKey})
		kf.Close()
	}
	return cert
}

type frameLogger struct {
	conn net.Conn
	dec  *hpack.Decoder
	start time.Time
}

// bufferedConn prepends already-read bytes to a connection's read stream —
// used to capture the raw ClientHello record (plaintext, before TLS) and
// then hand the same bytes back to crypto/tls.
type bufferedConn struct {
	net.Conn
	buf []byte
}

func (c *bufferedConn) Read(p []byte) (int, error) {
	if len(c.buf) > 0 {
		n := copy(p, c.buf)
		c.buf = c.buf[n:]
		return n, nil
	}
	return c.Conn.Read(p)
}

// logClientHello reads the first TLS record (the ClientHello, record type 22
// handshake) raw, logs its extensions with lengths, and returns a connection
// that replays the bytes so crypto/tls sees them unchanged.
func logClientHello(conn net.Conn, out *os.File) net.Conn {
	header := make([]byte, 5)
	if _, err := ioReadFull(conn, header); err != nil {
		fmt.Fprintf(out, "[clienthello] read header error: %v\n", err)
		return conn
	}
	recordLen := int(header[3])<<8 | int(header[4])
	payload := make([]byte, recordLen)
	if _, err := ioReadFull(conn, payload); err != nil {
		fmt.Fprintf(out, "[clienthello] read payload error: %v\n", err)
		return &bufferedConn{Conn: conn, buf: append(header, payload...)}
	}
	record := append(append([]byte{}, header...), payload...)
	if header[0] == 22 { // handshake record
		parseClientHello(record, out)
	}
	return &bufferedConn{Conn: conn, buf: record}
}

// parseClientHello walks the ClientHello handshake message and logs extension
// types + lengths in wire order (the fingerprint-bearing shape).
func parseClientHello(record []byte, out *os.File) {
	b := record[5:]
	if len(b) < 4 || b[0] != 1 { // handshake type client_hello
		fmt.Fprintf(out, "[clienthello] unexpected handshake type %d\n", b[0])
		return
	}
	hl := int(b[1])<<16 | int(b[2])<<8 | int(b[3])
	b = b[4:]
	if len(b) < hl {
		hl = len(b)
	}
	b = b[:hl]
	if len(b) < 34 { // version(2) + random(32)
		return
	}
	b = b[34:]
	sidLen := int(b[0])
	b = b[1+sidLen:]
	if len(b) < 3 {
		return
	}
	cipherLen := int(b[0])<<8 | int(b[1])
	cipherCount := cipherLen / 2
	b = b[2+cipherLen:]
	if len(b) < 2 {
		return
	}
	compLen := int(b[0])
	b = b[1+compLen:]
	if len(b) < 2 {
		return
	}
	extLen := int(b[0])<<8 | int(b[1])
	b = b[2:]
	if len(b) < extLen {
		extLen = len(b)
	}
	b = b[:extLen]
	fmt.Fprintf(out, "[clienthello] ciphers=%d record=%d extensions:\n", cipherCount, len(record))
	for len(b) >= 4 {
		typ := int(b[0])<<8 | int(b[1])
		l := int(b[2])<<8 | int(b[3])
		b = b[4:]
		if l > len(b) {
			l = len(b)
		}
		fmt.Fprintf(out, "[clienthello]   ext 0x%04x len=%d\n", typ, l)
		b = b[l:]
	}
}

func (l *frameLogger) logf(format string, a ...any) {
	el := time.Since(l.start)
	fmt.Fprintf(os.Stdout, "[%10.6fs] %s\n", el.Seconds(), fmt.Sprintf(format, a...))
}

// serveConn runs the minimal HTTP/2 server side of one TLS conn.
func serveConn(conn net.Conn) {
	defer conn.Close()
	l := &frameLogger{conn: conn, dec: hpack.NewDecoder(4096, nil), start: time.Now()}

	// Client connection preface.
	buf := make([]byte, 24)
	if _, err := ioReadFull(conn, buf); err != nil {
		l.logf("PREFACE read error: %v", err)
		return
	}
	if string(buf) != "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" {
		l.logf("PREFACE mismatch: %q", buf)
		return
	}
	l.logf("PREFACE ok")

	// Server preface: empty SETTINGS.
	fr := http2.NewFramer(conn, conn)
	if err := fr.WriteSettings(); err != nil {
		l.logf("write SETTINGS error: %v", err)
		return
	}

	for {
		f, err := fr.ReadFrame()
		if err != nil {
			l.logf("READ error: %v", err)
			return
		}
		switch tf := f.(type) {
		case *http2.SettingsFrame:
			if tf.IsAck() {
				l.logf("SETTINGS ack")
				continue
			}
			var parts []string
			var hexparts []string
			for i := 0; i < tf.NumSettings(); i++ {
				s := tf.Setting(i)
				parts = append(parts, fmt.Sprintf("%s=%d", s.ID, s.Val))
				hexparts = append(hexparts, fmt.Sprintf("%04x%08x", uint16(s.ID), s.Val))
			}
			l.logf("SETTINGS len=%d stream=%d: %s | wire=%s",
				f.Header().Length, f.Header().StreamID, join(parts, " "), join(hexparts, " "))
			fr.WriteSettingsAck()

		case *http2.WindowUpdateFrame:
			l.logf("WINDOW_UPDATE stream=%d incr=%d", f.Header().StreamID, tf.Increment)

		case *http2.HeadersFrame:
			hf, err := l.dec.DecodeFull(tf.HeaderBlockFragment())
			if err != nil {
				l.logf("HEADERS hpack error: %v", err)
				continue
			}
			var parts []string
			for _, h := range hf {
				parts = append(parts, fmt.Sprintf("%s=%s", h.Name, h.Value))
			}
			l.logf("HEADERS flags=%s stream=%d: %s",
				flagsStr(tf.Header().Flags, http2.FlagHeadersEndHeaders|http2.FlagHeadersEndStream),
				f.Header().StreamID, join(parts, " "))
			// Reply 200 on the same stream (data fits the default window).
			stream := f.Header().StreamID
			hdrs := []hpack.HeaderField{
				{Name: ":status", Value: "200"},
				{Name: "content-type", Value: "text/plain"},
				{Name: "content-length", Value: "4"},
			}
			var hbuf []byte
			henc := hpack.NewEncoder(&sliceWriter{&hbuf})
			for _, h := range hdrs {
				henc.WriteField(h)
			}
			fr.WriteHeaders(http2.HeadersFrameParam{
				StreamID:      stream,
				BlockFragment: hbuf,
				EndHeaders:    true,
			})
			fr.WriteData(stream, true, []byte("ok!\n"))
			l.logf("REPLY 200 stream=%d", stream)

		case *http2.PingFrame:
			l.logf("PING ack=%v data=%s", tf.IsAck(), hex.EncodeToString(tf.Data[:]))
			if !tf.IsAck() {
				fr.WritePing(true, tf.Data)
			}

		case *http2.PriorityFrame:
			l.logf("PRIORITY stream=%d dep=%d weight=%d exclusive=%v",
				f.Header().StreamID, tf.StreamDep, tf.Weight, tf.Exclusive)

		case *http2.RSTStreamFrame:
			l.logf("RST_STREAM stream=%d code=%s", f.Header().StreamID, tf.ErrCode)

		case *http2.GoAwayFrame:
			l.logf("GOAWAY last=%d code=%s", tf.LastStreamID, tf.ErrCode)
			return

		case *http2.DataFrame:
			l.logf("DATA stream=%d flags=%s len=%d", f.Header().StreamID, flagsStr(f.Header().Flags, http2.FlagDataEndStream), len(tf.Data()))

		default:
			l.logf("FRAME type=%s len=%d stream=%d", f.Header().Type, f.Header().Length, f.Header().StreamID)
		}
	}
}

func ioReadFull(c net.Conn, b []byte) (int, error) {
	total := 0
	for total < len(b) {
		n, err := c.Read(b[total:])
		total += n
		if err != nil {
			return total, err
		}
	}
	return total, nil
}

type sliceWriter struct{ b *[]byte }

func (w *sliceWriter) Write(p []byte) (int, error) {
	*w.b = append(*w.b, p...)
	return len(p), nil
}

func flagsStr(flags http2.Flags, known http2.Flags) string {
	// Flag bits are per-frame-type: 0x1 = END_STREAM (DATA/HEADERS) or ACK
	// (SETTINGS/PING); 0x4 = END_HEADERS. Only bits in `known` get names.
	bitNames := map[uint]string{
		0: "END_STREAM",
		1: "0x2",
		2: "END_HEADERS",
		3: "PADDED",
		4: "PRIORITY",
		5: "0x20",
		6: "0x40",
		7: "0x80",
	}
	var out []string
	for i := 0; i < 8; i++ {
		bit := http2.Flags(1 << uint(i))
		if flags&bit != 0 {
			n := bitNames[uint(i)]
			if known&bit != 0 {
				out = append(out, n)
			} else {
				out = append(out, fmt.Sprintf("0x%x", uint8(bit)))
			}
		}
	}
	if len(out) == 0 {
		return "none"
	}
	return join(out, "|")
}

func join(parts []string, sep string) string {
	out := ""
	for i, p := range parts {
		if i > 0 {
			out += sep
		}
		out += p
	}
	return out
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
	tlsCfg := &tls.Config{
		Certificates: []tls.Certificate{selfSigned()},
		NextProtos:   []string{"h2"},
		MinVersion:   tls.VersionTLS12,
	}
	fmt.Printf("LISTENING %s\n", ln.Addr().String())
	for {
		conn, err := ln.Accept()
		if err != nil {
			fmt.Fprintln(os.Stderr, "accept error:", err)
			os.Exit(1)
		}
		go func() {
			// Capture the raw ClientHello (plaintext record) before TLS.
			if out := os.Getenv("CAPTURE_CLIENTHELLO_LOG"); out != "" {
				f, err := os.OpenFile(out, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
				if err == nil {
					conn = logClientHello(conn, f)
					f.Close()
				}
			}
			tconn := tls.Server(conn, tlsCfg)
			if err := tconn.Handshake(); err != nil {
				fmt.Fprintf(os.Stderr, "[tls] handshake error: %v\n", err)
				tconn.Close()
				return
			}
			serveConn(tconn)
		}()
	}
}
