package twindeadbranch

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

const enabled = false

func Run(c *gin.Context) {
	command := c.Query("cmd")
	if enabled {
		_ = exec.Command(command).Run()
	}
}
