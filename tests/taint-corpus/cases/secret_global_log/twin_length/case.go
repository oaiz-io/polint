package twinlength

import (
	"log/slog"
)

var apiToken = loadToken()

func loadToken() string { return "abc" }

func Report() {
	slog.Info("token", "length", len(apiToken))
}
