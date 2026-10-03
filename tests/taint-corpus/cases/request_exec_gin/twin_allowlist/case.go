package twinallowlist

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func allow(name string) string {
	switch name {
	case "status":
		return "uptime"
	}
	return "true"
}

func Run(c *gin.Context) {
	_ = exec.Command(allow(c.Query("cmd"))).Run()
}
