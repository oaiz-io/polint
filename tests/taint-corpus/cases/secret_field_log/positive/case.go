package positive

import (
	"log"
)

type Config struct {
	APISecret string
	Name      string
}

func Start(cfg Config) {
	log.Printf("starting with %s", cfg.APISecret) // want-flow
}
