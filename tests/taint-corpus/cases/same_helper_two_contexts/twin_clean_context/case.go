package twincleancontext

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func same(value string) string { return value }

func Run(c *gin.Context) {
	_ = same(c.Query("cmd"))
	_ = exec.Command(same("uptime")).Run()
}
