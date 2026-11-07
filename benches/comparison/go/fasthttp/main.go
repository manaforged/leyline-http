package main

import (
	"bytes"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"
	"sort"
	"strconv"
	"sync"
	"time"

	"github.com/valyala/fasthttp"

	"leyline-comparison/cmpheaders"
	"leyline-comparison/cmptimeout"
)

var expected = []byte("ok-10byte!")

var poolSize = 64

func newClient() *fasthttp.Client {
	tlsConfig := &tls.Config{}
	if caPath := os.Getenv("CMP_CA"); caPath != "" {
		der, err := os.ReadFile(caPath)
		if err != nil {
			panic(err)
		}
		cert, err := x509.ParseCertificate(der)
		if err != nil {
			panic(err)
		}
		pool := x509.NewCertPool()
		pool.AddCert(cert)
		tlsConfig.RootCAs = pool
	} else {
		tlsConfig.InsecureSkipVerify = true
	}
	c := &fasthttp.Client{
		TLSConfig:       tlsConfig,
		MaxConnsPerHost: poolSize,
	}
	if cmptimeout.Default {
		c.ReadTimeout = 30 * time.Second
		c.WriteTimeout = 30 * time.Second
	}
	return c
}

func send(c *fasthttp.Client, req *fasthttp.Request, resp *fasthttp.Response) error {
	if cmptimeout.Total > 0 {
		return c.DoTimeout(req, resp, cmptimeout.Total)
	}
	return c.Do(req, resp)
}

func get(c *fasthttp.Client, url string) (int, []byte, []byte) {
	req := fasthttp.AcquireRequest()
	resp := fasthttp.AcquireResponse()
	defer fasthttp.ReleaseRequest(req)
	defer fasthttp.ReleaseResponse(resp)
	req.SetRequestURI(url)
	req.Header.SetMethod(fasthttp.MethodGet)
	for _, h := range cmpheaders.Pairs {
		req.Header.Set(h[0], h[1])
	}
	if err := send(c, req, resp); err != nil {
		panic(err)
	}
	return resp.StatusCode(), append([]byte(nil), resp.Header.Protocol()...), append([]byte(nil), resp.Body()...)
}

func do(c *fasthttp.Client, url string) {
	req := fasthttp.AcquireRequest()
	resp := fasthttp.AcquireResponse()
	req.SetRequestURI(url)
	req.Header.SetMethod(fasthttp.MethodGet)
	for _, h := range cmpheaders.Pairs {
		req.Header.Set(h[0], h[1])
	}
	if err := send(c, req, resp); err != nil {
		panic(err)
	}
	if resp.StatusCode() != fasthttp.StatusOK {
		panic(fmt.Sprintf("status %d", resp.StatusCode()))
	}
	if !bytes.Equal(resp.Header.Protocol(), []byte("HTTP/1.1")) {
		panic(fmt.Sprintf("protocol %s", resp.Header.Protocol()))
	}
	if !bytes.Equal(resp.Body(), expected) {
		panic("response body mismatch")
	}
	fasthttp.ReleaseRequest(req)
	fasthttp.ReleaseResponse(resp)
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
	if os.Getenv("CMP_PROTO") != "h1" {
		fmt.Fprintln(os.Stderr, "fasthttp speaks only HTTP/1.1; set CMP_PROTO=h1")
		os.Exit(1)
	}
	url := "https://127.0.0.1:8443/"
	warmN, coldN, concN, concC := 2000, 200, 20000, 64
	if len(os.Args) > 1 {
		url = os.Args[1]
	}
	if len(os.Args) > 2 && os.Args[2] == "equiv" {
		status, _, b := get(newClient(), url)
		fmt.Printf("EQUIV fasthttp status=%d fnv=%016x len=%d\n", status, fnv1a(b), len(b))
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

	clients := make([]*fasthttp.Client, connections)
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

	fmt.Printf("RESULT fasthttp connections=%d warm_rps=%.3f conc_rps=%.3f cold_rps=%.3f warm_p50_us=%d warm_p90_us=%d warm_p99_us=%d conc_p50_us=%d conc_p90_us=%d conc_p99_us=%d conc_p999_us=%d\n",
		connections, warmRps, concRps, coldRps,
		percentile(warm, 0.50), percentile(warm, 0.90), percentile(warm, 0.99),
		percentile(conc, 0.50), percentile(conc, 0.90), percentile(conc, 0.99), percentile(conc, 0.999))
}
