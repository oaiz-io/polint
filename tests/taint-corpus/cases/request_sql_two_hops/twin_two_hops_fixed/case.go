package twintwohopsfixed

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) query(text string) { h.db.Raw(text) }

func (h *Handler) forward(text string) {
	_ = text
	h.query("SELECT id FROM items")
}

func (h *Handler) Get(c *gin.Context) { h.forward(c.Query("columns")) }
