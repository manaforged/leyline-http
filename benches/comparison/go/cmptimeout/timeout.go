package cmptimeout

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"
)

var Default, Total = load()

func load() (bool, time.Duration) {
	v := os.Getenv("CMP_TIMEOUT")
	switch v {
	case "", "default":
		return true, 0
	case "none":
		return false, 0
	}
	n, err := strconv.Atoi(strings.TrimSuffix(v, "s"))
	if !strings.HasSuffix(v, "s") || err != nil || n <= 0 {
		fmt.Fprintf(os.Stderr, "CMP_TIMEOUT must be default, none or <N>s, got %q\n", v)
		os.Exit(1)
	}
	return false, time.Duration(n) * time.Second
}
