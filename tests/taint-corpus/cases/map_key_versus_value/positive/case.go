package positive

import (
	"os/exec"

	"github.com/gin-gonic/gin"
)

func Run(c *gin.Context) {
	commands := map[string]string{}
	commands["run"] = c.Query("cmd")
	_ = exec.Command(commands["run"]).Run() // want-flow
}
