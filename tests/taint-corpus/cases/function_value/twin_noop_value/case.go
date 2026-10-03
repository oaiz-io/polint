package twinnoopvalue

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func runShell(command string) { _ = exec.Command(command).Run() }

func runNothing(string) {}

func Handle(c *gin.Context) {
	run := runNothing
	run(c.Query("cmd"))
}

func Other() { runShell("uptime") }
