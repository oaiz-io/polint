package twinothermap

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) Get(c *gin.Context) {
	params := map[string]string{}
	params["id"] = c.Query("id")
	fixed := map[string]string{"id": "1"}
	_ = params
	h.db.Raw("SELECT * FROM items WHERE id = " + fixed["id"])
}
