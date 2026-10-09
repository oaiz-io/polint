package twinhelperparses

import (
	"fmt"
	"strconv"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func buildQuery(filter string) string {
	limit, _ := strconv.Atoi(filter)
	return fmt.Sprintf("SELECT * FROM items LIMIT %d", limit)
}

func (h *Handler) Search(c *gin.Context) {
	h.db.Raw(buildQuery(c.Query("filter")))
}
