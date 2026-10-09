package twinfixed

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Purge(c *gin.Context) {
	name := c.Query("name")
	_ = name
	go func() {
		h.db.Exec("DELETE FROM items WHERE name = 'archived'")
	}()
}
