package positive

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Handler struct{ db *gorm.DB }

func (h *Handler) query(text string) { h.db.Raw(text) } // want-flow

func (h *Handler) forward(text string) { h.query("SELECT " + text) }

func (h *Handler) Get(c *gin.Context) { h.forward(c.Query("columns")) }
