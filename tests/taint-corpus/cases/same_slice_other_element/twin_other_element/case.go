package twinotherelement

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	commands := []string{"uptime", c.Query("cmd")}
	_ = exec.Command(commands[0]).Run()
}
