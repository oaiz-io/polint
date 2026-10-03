package twinrunnerfromcaller

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

type Runner interface{ Run(command string) }

type shell struct{}

func (shell) Run(command string) { _ = exec.Command(command).Run() }

type quiet struct{}

func (quiet) Run(string) {}

// Handle gets its runner from a caller outside the program.
func Handle(runner Runner, c *gin.Context) { runner.Run(c.Query("cmd")) }

func Uses() {
	var r Runner = shell{}
	r.Run("uptime")
	Handle(quiet{}, nil)
}
