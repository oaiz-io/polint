package twincontextfixed

import (
	"context"

	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

type key struct{}

func (s *Store) List(ctx context.Context, actor Actor) {
	_ = actor
	ctx = context.WithValue(ctx, key{}, "all")
	tenant, _ := ctx.Value(key{}).(string)
	s.db.Where("tenant_id = ?", tenant).Find(&[]string{})
}
