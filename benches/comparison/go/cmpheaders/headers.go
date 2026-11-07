package cmpheaders

import (
	"os"
	"strings"
)

var Pairs = load()

func load() [][2]string {
	path := os.Getenv("CMP_HEADERS")
	if path == "" {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		panic(err)
	}
	var pairs [][2]string
	for _, line := range strings.Split(strings.TrimRight(string(data), "\n"), "\n") {
		name, value, ok := strings.Cut(line, "\t")
		if !ok {
			panic("header line needs a tab: " + line)
		}
		pairs = append(pairs, [2]string{name, value})
	}
	return pairs
}
