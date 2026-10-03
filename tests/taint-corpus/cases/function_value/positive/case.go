package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func runShell(command string) {
	_ = exec.Command(command).Run() // want-flow
}

func runNothing(string) {}

func Handle(c *gin.Context) {
	run := runShell
	run(c.Query("cmd"))
}

func Other() { runNothing("x") }
