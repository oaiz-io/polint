package twinbindparameter

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Get(c *gin.Context) {
	h.db.Raw("SELECT * FROM items WHERE id = ?", c.Param("id"))
}
