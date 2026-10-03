package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func same(value string) string { return value }

func Run(c *gin.Context) {
	_ = same("uptime")
	_ = exec.Command(same(c.Query("cmd"))).Run() // want-flow
}
