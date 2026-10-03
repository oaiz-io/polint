package positive

import (
	"log/slog"

	"github.com/gin-gonic/gin"
)

func Audit(c *gin.Context) {
	slog.Info("request", "agent", c.GetHeader("User-Agent")) // want-flow
}
