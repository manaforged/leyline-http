// Package cmppaced runs an open-loop request schedule and reports latency from each request's scheduled time.
package cmppaced

import (
	"fmt"
	"os"
	"slices"
	"strconv"
	"sync"
	"time"
)

const (
	lead          = 50 * time.Millisecond
	warmupSeconds = 1
	minGap        = 2 * time.Millisecond
)

// Plan is an open-loop schedule: Workers senders share Rate requests per second for a 1 s warm-up plus Seconds.
type Plan struct {
	Rate, Seconds, Workers int
}

type sample struct {
	service, lag uint32
}

type batch struct {
	last    time.Time
	samples []sample
}

// FromArgs reads "<url> paced <rate> <seconds> <workers>" from the command line.
func FromArgs(args []string) (Plan, bool) {
	if len(args) < 3 || args[2] != "paced" {
		return Plan{}, false
	}
	field := func(i int, name string) int {
		if len(args) > i {
			if n, err := strconv.Atoi(args[i]); err == nil && n > 0 {
				return n
			}
		}
		fmt.Fprintf(os.Stderr, "paced needs a positive %s\n", name)
		os.Exit(1)
		return 0
	}
	p := Plan{Rate: field(3, "rate"), Seconds: field(4, "seconds"), Workers: field(5, "worker count")}
	if p.gap() < minGap {
		fmt.Fprintf(os.Stderr, "paced needs at least %v between one worker's requests; raise the worker count or lower the rate\n", minGap)
		os.Exit(1)
	}
	return p, true
}

func (p Plan) gap() time.Duration {
	return time.Duration(float64(time.Second) * float64(p.Workers) / float64(p.Rate))
}

// Run sends the schedule from one goroutine per worker, calls one(worker) for each request, and prints the report.
func (p Plan) Run(one func(worker int)) {
	per := p.Rate * (p.Seconds + warmupSeconds) / p.Workers
	measuredCap := p.Rate*p.Seconds/p.Workers + 1
	gap := p.gap()
	start := time.Now().Add(lead)
	first := start.Add(warmupSeconds * time.Second)
	batches := make([]batch, p.Workers)
	var wg sync.WaitGroup
	for w := 0; w < p.Workers; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			offset := time.Duration(float64(gap) * float64(w) / float64(p.Workers))
			out := make([]sample, 0, measuredCap)
			ready := start
			last := first
			for i := 0; i < per; i++ {
				intended := start.Add(offset + time.Duration(i)*gap)
				time.Sleep(time.Until(intended))
				sent := time.Now()
				one(w)
				done := time.Now()
				if !intended.Before(first) {
					if done.After(last) {
						last = done
					}
					origin := sent
					if ready.After(intended) {
						origin = intended
					}
					svc := done.Sub(sent).Microseconds()
					if svc < 0 {
						svc = 0
					}
					lag := sent.Sub(origin).Microseconds()
					if lag < 0 {
						lag = 0
					}
					out = append(out, sample{service: uint32(svc), lag: uint32(lag)})
				}
				ready = done
			}
			batches[w] = batch{last: last, samples: out}
		}(w)
	}
	wg.Wait()
	report(p, first, batches)
}

func pct(v []uint32, q float64) uint32 {
	return v[int(float64(len(v)-1)*q+0.5)]
}

func report(p Plan, first time.Time, batches []batch) {
	total := 0
	last := first
	for _, b := range batches {
		total += len(b.samples)
		if b.last.After(last) {
			last = b.last
		}
	}
	if total == 0 {
		fmt.Fprintln(os.Stderr, "paced run measured no requests after the warm-up")
		os.Exit(1)
	}
	buf := make([]uint32, 0, total)
	for _, b := range batches {
		for _, s := range b.samples {
			buf = append(buf, s.service)
		}
	}
	slices.Sort(buf)
	svc50, svc99, svc999, svcMax := pct(buf, 0.50), pct(buf, 0.99), pct(buf, 0.999), buf[len(buf)-1]

	buf = buf[:0]
	for _, b := range batches {
		for _, s := range b.samples {
			buf = append(buf, s.lag)
		}
	}
	slices.Sort(buf)
	lag50, lag999 := pct(buf, 0.50), pct(buf, 0.999)

	buf = buf[:0]
	for _, b := range batches {
		for _, s := range b.samples {
			buf = append(buf, s.service+s.lag)
		}
	}
	slices.Sort(buf)
	corr50, corr99, corr999 := pct(buf, 0.50), pct(buf, 0.99), pct(buf, 0.999)

	elapsed := last.Sub(first).Seconds()
	fmt.Printf("PACED rate=%d completed=%d elapsed_s=%.3f rps=%.1f service_p50_us=%d service_p99_us=%d service_p999_us=%d service_max_us=%d lag_p50_us=%d lag_p999_us=%d corrected_p50_us=%d corrected_p99_us=%d corrected_p999_us=%d\n",
		p.Rate, total, elapsed, float64(total)/elapsed,
		svc50, svc99, svc999, svcMax, lag50, lag999,
		corr50, corr99, corr999)
}
