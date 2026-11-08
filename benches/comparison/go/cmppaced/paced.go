// Package cmppaced runs an open-loop request schedule and reports latency from each request's scheduled time.
package cmppaced

import (
	"fmt"
	"os"
	"sort"
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

type row struct {
	ready, intended, sent, done time.Time
}

func (r row) origin() time.Time {
	if r.ready.After(r.intended) {
		return r.intended
	}
	return r.sent
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
	gap := p.gap()
	start := time.Now().Add(lead)
	rows := make([][]row, p.Workers)
	var wg sync.WaitGroup
	for w := 0; w < p.Workers; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			offset := time.Duration(float64(gap) * float64(w) / float64(p.Workers))
			out := make([]row, 0, per)
			ready := start
			for i := 0; i < per; i++ {
				intended := start.Add(offset + time.Duration(i)*gap)
				time.Sleep(time.Until(intended))
				sent := time.Now()
				one(w)
				done := time.Now()
				out = append(out, row{ready, intended, sent, done})
				ready = done
			}
			rows[w] = out
		}(w)
	}
	wg.Wait()
	report(p, start, rows)
}

func report(p Plan, start time.Time, rows [][]row) {
	first := start.Add(warmupSeconds * time.Second)
	var all []row
	last := first
	for _, rs := range rows {
		for _, r := range rs {
			if r.intended.Before(first) {
				continue
			}
			all = append(all, r)
			if r.done.After(last) {
				last = r.done
			}
		}
	}
	if len(all) == 0 {
		fmt.Fprintln(os.Stderr, "paced run measured no requests after the warm-up")
		os.Exit(1)
	}
	column := func(f func(row) int64) []int64 {
		v := make([]int64, len(all))
		for i, r := range all {
			v[i] = f(r)
		}
		sort.Slice(v, func(i, j int) bool { return v[i] < v[j] })
		return v
	}
	pct := func(v []int64, q float64) int64 { return v[int(float64(len(v)-1)*q+0.5)] }
	service := func(r row) int64 { return r.done.Sub(r.sent).Microseconds() }
	lag := func(r row) int64 { return r.sent.Sub(r.origin()).Microseconds() }
	s, l := column(service), column(lag)
	c := column(func(r row) int64 { return r.done.Sub(r.origin()).Microseconds() })
	elapsed := last.Sub(first).Seconds()
	fmt.Printf("PACED rate=%d completed=%d elapsed_s=%.3f rps=%.1f service_p50_us=%d service_p99_us=%d service_p999_us=%d service_max_us=%d lag_p50_us=%d lag_p999_us=%d corrected_p50_us=%d corrected_p99_us=%d corrected_p999_us=%d\n",
		p.Rate, len(all), elapsed, float64(len(all))/elapsed,
		pct(s, 0.50), pct(s, 0.99), pct(s, 0.999), pct(s, 1.0), pct(l, 0.50), pct(l, 0.999),
		pct(c, 0.50), pct(c, 0.99), pct(c, 0.999))
}
