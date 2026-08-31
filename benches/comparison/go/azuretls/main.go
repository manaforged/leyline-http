// Noooste/azuretls-client (utls) Chrome head-to-head client.
package main

import (
	"fmt"
	"os"
	"strconv"
	"sync"
	"time"

	azuretls "github.com/Noooste/azuretls-client"
)

func newSession() *azuretls.Session {
	s := azuretls.NewSession()
	s.InsecureSkipVerify = true
	s.Browser = azuretls.Chrome
	return s
}

func do(s *azuretls.Session, url string) {
	if _, err := s.Get(url); err != nil {
		panic(err)
	}
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

	if len(os.Args) > 2 && os.Args[2] == "print" {
		resp, err := newSession().Get(url)
		if err != nil {
			panic(err)
		}
		fmt.Println(string(resp.Body))
		return
	}

	s := newSession()
	do(s, url)

	start := time.Now()
	for i := 0; i < warmN; i++ {
		do(s, url)
	}
	warmRps := float64(warmN) / time.Since(start).Seconds()

	per := concN / concC
	start = time.Now()
	var wg sync.WaitGroup
	for w := 0; w < concC; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := 0; i < per; i++ {
				do(s, url)
			}
		}()
	}
	wg.Wait()
	concRps := float64(per*concC) / time.Since(start).Seconds()

	start = time.Now()
	for i := 0; i < coldN; i++ {
		fresh := newSession()
		do(fresh, url)
		fresh.Close()
	}
	coldRps := float64(coldN) / time.Since(start).Seconds()

	fmt.Printf("RESULT azuretls warm_rps=%.0f conc_rps=%.0f cold_rps=%.0f\n", warmRps, concRps, coldRps)
}
