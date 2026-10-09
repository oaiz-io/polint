package positive

import (
	"log"
)

type holder struct{ Value string }

func Show(token string) {
	var h holder
	h.Value = token
	log.Println(h.Value) // want-flow
}
