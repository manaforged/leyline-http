package main

import (
	"bytes"
	"crypto/x509"
	"fmt"
	"io"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"leyline-comparison/cmpheaders"
	"leyline-comparison/cmptimeout"

	http "github.com/bogdanfinn/fhttp"
	tls_client "github.com/bogdanfinn/tls-client"
	"github.com/bogdanfinn/tls-client/profiles"
)

var expected = []byte("ok-10byte!")

var h1 = os.Getenv("CMP_PROTO") == "h1"

var poolSize = 64

var profile = func() profiles.ClientProfile {
	version := os.Getenv("CMP_CHROME")
	if version == "" {
		version = "152"
	}
	p, ok := profiles.MappedTLSClients["chrome_"+version]
	if !ok {
		fmt.Fprintf(os.Stderr, "tls-client has no chrome_%s profile\n", version)
		os.Exit(1)
	}
	return p
}()

func newClient() tls_client.HttpClient {
	opts := []tls_client.HttpClientOption{
		tls_client.WithClientProfile(profile),
	}
	if cmptimeout.Default {
		opts = append(opts, tls_client.WithTimeoutSeconds(30))
	} else {
		opts = append(opts, tls_client.WithTimeoutMilliseconds(int(cmptimeout.Total.Milliseconds())))
	}
	transport := &tls_client.TransportOptions{}
	if h1 {
		opts = append(opts, tls_client.WithForceHttp1())
		transport.MaxIdleConns = poolSize
		transport.MaxIdleConnsPerHost = poolSize
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
		transport.RootCAs = pool
	} else {
		opts = append(opts, tls_client.WithInsecureSkipVerify())
	}
	opts = append(opts, tls_client.WithTransportOptions(transport))
	c, err := tls_client.NewHttpClient(tls_client.NewNoopLogger(), opts...)
	if err != nil {
		panic(err)
	}
	return c
}

func newRequest(url string) *http.Request {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		panic(err)
	}
	if len(cmpheaders.Pairs) > 0 {
		order := make([]string, 0, len(cmpheaders.Pairs))
		for _, h := range cmpheaders.Pairs {
			req.Header.Add(h[0], h[1])
			order = append(order, strings.ToLower(h[0]))
		}
		req.Header[http.HeaderOrderKey] = order
	}
	return req
}

func do(c tls_client.HttpClient, url string) {
	req := newRequest(url)
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
	want := 2
	if h1 {
		want = 1
	}
	if resp.ProtoMajor != want {
		panic(fmt.Sprintf("protocol %s", resp.Proto))
	}
	if !bytes.Equal(body, expected) {
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

func percentile(sorted []int64, p float64) int64 {
	return sorted[int(float64(len(sorted)-1)*p+0.5)]
}

func main() {
	if p := os.Getenv("CMP_PROTO"); p != "" && p != "h1" && p != "h2" {
		fmt.Fprintf(os.Stderr, "unknown CMP_PROTO %q\n", p)
		os.Exit(1)
	}
	url := "https://127.0.0.1:8443/"
	warmN, coldN, concN, concC := 2000, 200, 20000, 64
	if len(os.Args) > 1 && os.Args[1] == "profiles" {
		var versions []int
		for name := range profiles.MappedTLSClients {
			if v, err := strconv.Atoi(strings.TrimPrefix(name, "chrome_")); err == nil && strings.HasPrefix(name, "chrome_") {
				versions = append(versions, v)
			}
		}
		sort.Ints(versions)
		for _, v := range versions {
			fmt.Println(v)
		}
		return
	}
	if len(os.Args) > 1 {
		url = os.Args[1]
	}
	if len(os.Args) > 2 && os.Args[2] == "equiv" {
		resp, err := newClient().Do(newRequest(url))
		if err != nil {
			panic(err)
		}
		b, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		fmt.Printf("EQUIV tls-client status=%d fnv=%016x len=%d\n", resp.StatusCode, fnv1a(b), len(b))
		return
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
	if concC > poolSize {
		poolSize = concC
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

	clients := make([]tls_client.HttpClient, connections)
	for i := range clients {
		clients[i] = newClient()
		do(clients[i], url)
	}

	warm := make([]int64, 0, warmN)
	start := time.Now()
	for i := 0; i < warmN; i++ {
		t0 := time.Now()
		do(clients[0], url)
		warm = append(warm, time.Since(t0).Microseconds())
	}
	warmRps := float64(warmN) / time.Since(start).Seconds()
	sort.Slice(warm, func(i, j int) bool { return warm[i] < warm[j] })

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

	conc := make([]int64, 0, per*concC)
	for _, l := range lat {
		conc = append(conc, l...)
	}
	sort.Slice(conc, func(i, j int) bool { return conc[i] < conc[j] })

	start = time.Now()
	for i := 0; i < coldN; i++ {
		fresh := newClient()
		do(fresh, url)
		fresh.CloseIdleConnections()
	}
	coldRps := float64(coldN) / time.Since(start).Seconds()

	fmt.Printf("RESULT tls-client connections=%d warm_rps=%.3f conc_rps=%.3f cold_rps=%.3f warm_p50_us=%d warm_p90_us=%d warm_p99_us=%d conc_p50_us=%d conc_p90_us=%d conc_p99_us=%d conc_p999_us=%d\n",
		connections, warmRps, concRps, coldRps,
		percentile(warm, 0.50), percentile(warm, 0.90), percentile(warm, 0.99),
		percentile(conc, 0.50), percentile(conc, 0.90), percentile(conc, 0.99), percentile(conc, 0.999))
}
