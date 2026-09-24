// bogdanfinn/tls-client (utls) Chrome-146 head-to-head client.
package main

import (
	"bytes"
	"crypto/x509"
	"fmt"
	"io"
	"os"
	"sort"
	"strconv"
	"sync"
	"time"

	http "github.com/bogdanfinn/fhttp"
	tls_client "github.com/bogdanfinn/tls-client"
	"github.com/bogdanfinn/tls-client/profiles"
)

var expected []byte

func newClient() tls_client.HttpClient {
	opts := []tls_client.HttpClientOption{
		tls_client.WithClientProfile(profiles.Chrome_152),
		tls_client.WithTimeoutSeconds(30),
	}
	if caPath := os.Getenv("CMP_CA"); caPath != "" {
		der, err := os.ReadFile(caPath)
		if err != nil {
			panic(err)
		}
		pool := x509.NewCertPool()
		if !pool.AppendCertsFromPEM(der) {
			cert, err := x509.ParseCertificate(der)
			if err != nil {
				panic(err)
			}
			pool.AddCert(cert)
		}
		opts = append(opts, tls_client.WithTransportOptions(&tls_client.TransportOptions{RootCAs: pool}))
	} else {
		opts = append(opts, tls_client.WithInsecureSkipVerify())
	}
	c, err := tls_client.NewHttpClient(tls_client.NewNoopLogger(), opts...)
	if err != nil {
		panic(err)
	}
	return c
}

func do(c tls_client.HttpClient, url string) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		panic(err)
	}
	resp, err := c.Do(req)
	if err != nil {
		panic(err)
	}
	body, err := io.ReadAll(resp.Body)
	resp.Body.Close()
	if err != nil {
		panic(err)
	}
	if resp.StatusCode != 200 {
		panic(fmt.Sprintf("status %d", resp.StatusCode))
	}
	if expected != nil && !bytes.Equal(body, expected) {
		panic("response body mismatch")
	}
}

func fnv1a(b []byte) uint64 {
	h := uint64(0xcbf29ce484222325)
	for _, x := range b {
		h ^= uint64(x)
		h *= 0x00000100000001b3
	}
	return h
}

func main() {
	url := "https://127.0.0.1:8443/"
	warmN, coldN, concN, concC := 2000, 200, 20000, 64
	if len(os.Args) > 1 {
		url = os.Args[1]
	}
	if len(os.Args) > 2 {
		warmN, _ = strconv.Atoi(os.Args[2])
	}
	if len(os.Args) > 3 {
		coldN, _ = strconv.Atoi(os.Args[3])
	}
	if len(os.Args) > 4 {
		concN, _ = strconv.Atoi(os.Args[4])
	}
	if len(os.Args) > 5 {
		concC, _ = strconv.Atoi(os.Args[5])
	}

	if n, err := strconv.Atoi(os.Getenv("CMP_BODY")); err == nil {
		expected = make([]byte, n)
		for i := range expected {
			expected[i] = byte(i % 251)
		}
	}
	connections := 1
	if n, err := strconv.Atoi(os.Getenv("CMP_CONNECTIONS")); err == nil && n > 0 {
		connections = n
	}

	if len(os.Args) > 2 && os.Args[2] == "print" {
		req, _ := http.NewRequest(http.MethodGet, url, nil)
		resp, err := newClient().Do(req)
		if err != nil {
			panic(err)
		}
		b, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		fmt.Println(string(b))
		return
	}

	if len(os.Args) > 2 && os.Args[2] == "equiv" {
		req, _ := http.NewRequest(http.MethodGet, url, nil)
		resp, err := newClient().Do(req)
		if err != nil {
			panic(err)
		}
		b, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		fmt.Printf("EQUIV tls-client status=%d fnv=%016x len=%d\n", resp.StatusCode, fnv1a(b), len(b))
		return
	}

	clients := make([]tls_client.HttpClient, connections)
	for i := range clients {
		clients[i] = newClient()
	}
	do(clients[0], url)

	start := time.Now()
	for i := 0; i < warmN; i++ {
		do(clients[0], url)
	}
	warmRps := float64(warmN) / time.Since(start).Seconds()

	per := concN / concC
	lat := make([][]int64, concC)
	start = time.Now()
	var wg sync.WaitGroup
	for w := 0; w < concC; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			c := clients[w%connections]
			l := make([]int64, 0, per)
			for i := 0; i < per; i++ {
				t0 := time.Now()
				do(c, url)
				l = append(l, time.Since(t0).Microseconds())
			}
			lat[w] = l
		}(w)
	}
	wg.Wait()
	concRps := float64(per*concC) / time.Since(start).Seconds()

	all := make([]int64, 0, per*concC)
	for _, l := range lat {
		all = append(all, l...)
	}
	sort.Slice(all, func(i, j int) bool { return all[i] < all[j] })
	pct := func(p float64) int64 { return all[int(float64(len(all)-1)*p+0.5)] }

	start = time.Now()
	for i := 0; i < coldN; i++ {
		fresh := newClient()
		do(fresh, url)
		fresh.CloseIdleConnections()
	}
	coldRps := float64(coldN) / time.Since(start).Seconds()

	fmt.Printf("RESULT tls-client connections=%d warm_rps=%.0f conc_rps=%.0f cold_rps=%.0f conc_p50_us=%d conc_p90_us=%d conc_p99_us=%d conc_p999_us=%d\n",
		connections, warmRps, concRps, coldRps, pct(0.50), pct(0.90), pct(0.99), pct(0.999))
}
