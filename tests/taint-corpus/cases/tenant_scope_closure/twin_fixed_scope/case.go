package twinfixedscope

import (
	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

func tenantScope(tenant string) func(*gorm.DB) *gorm.DB {
	return func(db *gorm.DB) *gorm.DB {
		return db.Where("tenant_id = ?", tenant)
	}
}

func (s *Store) List(actor Actor) {
	_ = actor
	s.db.Scopes(tenantScope("all")).Find(&[]string{})
}
