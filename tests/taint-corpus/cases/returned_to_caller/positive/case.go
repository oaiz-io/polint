package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func input(c *gin.Context) string { return c.Query("cmd") }

func Run(c *gin.Context) {
	_ = exec.Command(input(c)).Run() // want-flow
}
