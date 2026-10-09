package twinotherglobal

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

var last, other string

func Store(c *gin.Context) { last = c.Query("cmd") }

func Replay() {
	_ = exec.Command(other).Run()
}
