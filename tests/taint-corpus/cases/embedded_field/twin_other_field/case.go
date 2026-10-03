package twinotherfield

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

type base struct{ Command string }

type job struct {
	base
	Name string
}

func Run(c *gin.Context) {
	var j job
	j.Name = "nightly"
	j.Command = c.Query("cmd")
	_ = exec.Command(j.Name).Run()
}
