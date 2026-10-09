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

type Handler struct{ runner Runner }

func New() *Handler { return &Handler{runner: shell{}} }

func (h *Handler) Handle(c *gin.Context) { h.runner.Run(c.Query("cmd")) }
