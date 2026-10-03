package twinotherfield

import (
	"log"
)

type Config struct {
	APISecret string
	Name      string
}

func Start(cfg Config) {
	log.Printf("starting %s", cfg.Name)
}
