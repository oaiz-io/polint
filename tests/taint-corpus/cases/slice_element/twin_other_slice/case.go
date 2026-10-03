package twinotherslice

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	commands := []string{c.Query("cmd")}
	fixed := []string{"uptime"}
	_ = commands
	_ = exec.Command(fixed[0]).Run()
}
