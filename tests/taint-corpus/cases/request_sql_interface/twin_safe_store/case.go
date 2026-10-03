package twinsafestore

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Store interface{ FindByName(name string) }

type Handler struct{ store Store }

type safeStore struct{ db *gorm.DB }

func (s *safeStore) FindByName(name string) { s.db.Where("name = ?", name) }

func New(db *gorm.DB) *Handler { return &Handler{store: &safeStore{db: db}} }

func (h *Handler) Get(c *gin.Context) { h.store.FindByName(c.Query("name")) }
