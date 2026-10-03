package twinconstantset

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

type box struct{ value string }

func (b *box) Set(value string) { b.value = value }

func (b *box) Get() string { return b.value }

func Run(c *gin.Context) {
	var b box
	_ = c.Query("cmd")
	b.Set("uptime")
	_ = exec.Command(b.Get()).Run()
}
