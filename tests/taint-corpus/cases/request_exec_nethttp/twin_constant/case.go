package twinconstant

import (
	"net/http"
	"os/exec"
)

func Handle(w http.ResponseWriter, r *http.Request) {
	_ = r.URL.Query().Get("file")
	_ = exec.Command("cat", "/etc/hostname").Run()
}
