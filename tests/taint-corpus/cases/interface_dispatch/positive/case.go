package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

type Runner interface{ Run(command string) }

type shell struct{}

func (shell) Run(command string) {
	_ = exec.Command(command).Run() // want-flow
}

type noop struct{}

func (noop) Run(string) {}

func Handle(c *gin.Context) {
	var runner Runner = shell{}
	runner.Run(c.Query("cmd"))
}
