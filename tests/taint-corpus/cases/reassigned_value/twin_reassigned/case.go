package twinreassigned

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	command := c.Query("cmd")
	_ = command
	command = "uptime"
	_ = exec.Command(command).Run()
}
