package twinotherfield

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

type filter struct {
	Column string
	Value  string
}

func (h *Handler) Sorted(c *gin.Context) {
	f := filter{Column: c.Query("column"), Value: "name"}
	h.db.Order(f.Value)
}
