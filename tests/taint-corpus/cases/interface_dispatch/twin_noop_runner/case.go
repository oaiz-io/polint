package twinnooprunner

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

type Runner interface{ Run(command string) }

type shell struct{}

func (shell) Run(command string) { _ = exec.Command(command).Run() }

type noop struct{}

func (noop) Run(string) {}

func Handle(c *gin.Context) {
	var runner Runner = noop{}
	runner.Run(c.Query("cmd"))
}

func Other() {
	var runner Runner = shell{}
	runner.Run("uptime")
}
