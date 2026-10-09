package positive

import (
	"net/http"
	"os/exec"
)

func Handle(w http.ResponseWriter, r *http.Request) {
	file := r.URL.Query().Get("file")
	_ = exec.Command("cat", file).Run() // want-flow
}
