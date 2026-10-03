package positive

import (
	"log/slog"
)

var apiToken = loadToken()

func loadToken() string { return "abc" }

func Report() {
	slog.Info("token", "value", apiToken) // want-flow
}
