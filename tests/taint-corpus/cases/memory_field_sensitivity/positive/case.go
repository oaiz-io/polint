package positive

import (
	"log"
)

type pair struct {
	Value string
	Label string
}

func Show(token string) {
	var p pair
	p.Label = "x"
	p.Value = token
	log.Println(p.Value) // want-flow
}
