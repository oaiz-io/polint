package positive

import (
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type Store interface{ FindByName(name string) }

type Handler struct{ store Store }

type sqlStore struct{ db *gorm.DB }

func (s *sqlStore) FindByName(name string) {
	s.db.Raw("SELECT * FROM items WHERE name = '" + name + "'") // want-flow
}

func New(db *gorm.DB) *Handler { return &Handler{store: &sqlStore{db: db}} }

func (h *Handler) Get(c *gin.Context) { h.store.FindByName(c.Query("name")) }
