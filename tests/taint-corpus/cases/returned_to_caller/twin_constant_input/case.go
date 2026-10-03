package twinconstantinput

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func input(c *gin.Context) string {
	_ = c.Query("cmd")
	return "uptime"
}

func Run(c *gin.Context) {
	_ = exec.Command(input(c)).Run()
}
