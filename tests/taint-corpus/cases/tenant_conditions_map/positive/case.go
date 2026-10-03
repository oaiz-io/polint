package positive

import (
	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

func conditions(actor Actor) map[string]any {
	return map[string]any{"tenant_id": actor.TenantID()}
}

func (s *Store) List(actor Actor) {
	s.db.Where(conditions(actor)).Find(&[]string{}) // want-flow
}
