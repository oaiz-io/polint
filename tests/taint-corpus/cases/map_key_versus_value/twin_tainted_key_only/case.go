package twintaintedkeyonly

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	commands := map[string]string{}
	commands[c.Query("name")] = "uptime"
	_ = exec.Command(commands["run"]).Run()
}
