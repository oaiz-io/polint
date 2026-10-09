package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	commands := []string{c.Query("cmd")}
	_ = exec.Command(commands[0]).Run() // want-flow
}
