package positive

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func buildQuery(filter string) string { return "SELECT * FROM items WHERE " + filter }

func (h *Handler) Search(c *gin.Context) {
	h.db.Raw(buildQuery(c.Query("filter"))) // want-flow
}
