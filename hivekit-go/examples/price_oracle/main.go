// price_oracle: sources submit prices for a pair; anyone reads the median.
// Uses persistent storage and events.
//
// Prices are integers in the pair's minor unit (e.g. cents): event data and
// stored state must not contain fractional numbers.
//
//	hivec build ./examples/price_oracle
//	hivec run -data-dir ./state dist/price_oracle.hbc submit '{"pair":"ETH/USD","source":"a","price":320012}'
//	hivec run -data-dir ./state dist/price_oracle.hbc price '{"pair":"ETH/USD"}'
package main

import (
	"errors"
	"sort"

	hivekit "github.com/Necter-Network/hivekit/hivekit-go"
)

type submission struct {
	Pair   string `json:"pair"`
	Source string `json:"source"`
	Price  int64  `json:"price"`
}

type query struct {
	Pair string `json:"pair"`
}

type quote struct {
	Pair    string `json:"pair"`
	Median  int64  `json:"median"`
	Sources int    `json:"sources"`
}

// book is the stored state of one pair: source → latest price.
type book map[string]int64

func key(pair string) string { return "pair:" + pair }

func load(pair string) (book, error) {
	b := book{}
	_, err := hivekit.StorageGetJSON(key(pair), &b)
	return b, err
}

func median(b book) int64 {
	prices := make([]int64, 0, len(b))
	for _, p := range b {
		prices = append(prices, p)
	}
	sort.Slice(prices, func(i, j int) bool { return prices[i] < prices[j] })
	n := len(prices)
	if n%2 == 1 {
		return prices[n/2]
	}
	return (prices[n/2-1] + prices[n/2]) / 2
}

func init() {
	hivekit.DefineJSON("submit", func(s submission) (quote, error) {
		if s.Pair == "" || s.Source == "" {
			return quote{}, errors.New("pair and source are required")
		}
		if s.Price <= 0 {
			return quote{}, errors.New("price must be a positive integer")
		}
		b, err := load(s.Pair)
		if err != nil {
			return quote{}, err
		}
		b[s.Source] = s.Price
		if err := hivekit.StorageSetJSON(key(s.Pair), b); err != nil {
			return quote{}, err
		}
		if err := hivekit.Emit("price.submitted", s); err != nil {
			return quote{}, err
		}
		return quote{Pair: s.Pair, Median: median(b), Sources: len(b)}, nil
	})

	hivekit.DefineJSON("price", func(q query) (quote, error) {
		b, err := load(q.Pair)
		if err != nil {
			return quote{}, err
		}
		if len(b) == 0 {
			return quote{}, errors.New("no prices for " + q.Pair)
		}
		return quote{Pair: q.Pair, Median: median(b), Sources: len(b)}, nil
	})

	hivekit.DefineJSON("reset", func(q query) (map[string]bool, error) {
		hivekit.StorageDel(key(q.Pair))
		return map[string]bool{"ok": true}, nil
	})
}

func main() {}
