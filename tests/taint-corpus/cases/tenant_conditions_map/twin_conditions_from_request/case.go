package twinconditionsfromrequest

import (
	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

func conditions(tenant string) map[string]any {
	return map[string]any{"tenant_id": tenant}
}

func (s *Store) List(actor Actor, tenant string) {
	_ = actor
	s.db.Where(conditions(tenant)).Find(&[]string{})
}
