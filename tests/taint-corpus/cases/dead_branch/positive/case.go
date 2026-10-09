package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	command := c.Query("cmd")
	if true {
		_ = exec.Command(command).Run() // want-flow
	}
}
