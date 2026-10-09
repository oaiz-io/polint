package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

var last string

func Store(c *gin.Context) { last = c.Query("cmd") }

func Replay() {
	_ = exec.Command(last).Run() // want-flow
}
