package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	_ = exec.Command(c.Query("cmd")).Run() // want-flow
}
