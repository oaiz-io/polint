package positive

import (
	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

func (s *Store) List(actor Actor) {
	tenant := actor.TenantID()
	s.db.Scopes(func(db *gorm.DB) *gorm.DB {
		return db.Where("tenant_id = ?", tenant) // want-flow
	}).Find(&[]string{})
}
