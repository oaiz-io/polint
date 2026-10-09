package positive

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

type request struct {
	Name  string
	Limit int
}

func (h *Handler) List(c *gin.Context) {
	var req request
	if err := c.ShouldBindJSON(&req); err != nil {
		return
	}
	h.db.Where("name = '" + req.Name + "'").Find(&[]string{}) // want-flow
}
