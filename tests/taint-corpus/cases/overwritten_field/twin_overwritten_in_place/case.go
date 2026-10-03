package twinoverwritteninplace

import (
	"log"
)

type holder struct{ Value string }

func Show(token string) {
	var h holder
	h.Value = token
	h.Value = "redacted"
	log.Println(h.Value)
}
