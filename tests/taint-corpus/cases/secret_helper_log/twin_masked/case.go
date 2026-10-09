package twinmasked

import (
	"log"
	"strings"
)

func describe(password string) string { return "pw=" + strings.Repeat("*", len(password)) }

func Login(password string) {
	log.Println(describe(password))
}
