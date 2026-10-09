package twinrequestvalue

import (
	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

func (s *Store) List(actor Actor, tenant string) {
	_ = actor
	s.db.Where("tenant_id = ?", tenant).Find(&[]string{})
}
