package positive

import (
	"fmt"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Get(c *gin.Context) {
	id := c.Param("id")
	h.db.Raw(fmt.Sprintf("SELECT * FROM items WHERE id = '%s'", id)) // want-flow
}
