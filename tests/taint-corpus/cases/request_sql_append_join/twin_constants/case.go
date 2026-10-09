package twinconstants

import (
	"strings"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Columns(c *gin.Context) {
	parts := []string{"id", "name"}
	_ = c.Query("column")
	h.db.Raw("SELECT " + strings.Join(parts, ",") + " FROM items")
}
