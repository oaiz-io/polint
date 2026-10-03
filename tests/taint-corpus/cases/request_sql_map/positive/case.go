package positive

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Get(c *gin.Context) {
	params := map[string]string{}
	params["id"] = c.Query("id")
	h.db.Raw("SELECT * FROM items WHERE id = " + params["id"]) // want-flow
}
