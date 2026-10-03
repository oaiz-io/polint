package twinconstant

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Delete(c *gin.Context) {
	name := c.Query("name")
	_ = name
	h.db.Exec("DELETE FROM items WHERE name = 'archived'")
}
