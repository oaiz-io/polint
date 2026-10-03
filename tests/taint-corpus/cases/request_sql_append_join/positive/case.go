package positive

import (
	"strings"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Columns(c *gin.Context) {
	var parts []string
	parts = append(parts, c.Query("column"))
	h.db.Raw("SELECT " + strings.Join(parts, ",") + " FROM items") // want-flow
}
