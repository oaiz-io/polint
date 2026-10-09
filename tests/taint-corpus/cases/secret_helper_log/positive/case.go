package positive

import (
	"log"
)

func describe(password string) string { return "pw=" + password }

func Login(password string) {
	log.Println(describe(password)) // want-flow
}
