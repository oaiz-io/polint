package twinstaticlog

import (
	"log/slog"

	"github.com/gin-gonic/gin"
)

func Audit(c *gin.Context) {
	_ = c.GetHeader("User-Agent")
	slog.Info("request", "agent", "redacted")
}
