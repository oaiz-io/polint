package positive

import (
	"context"

	"gorm.io/gorm"
)

type Actor struct{ tenant string }

func (a Actor) TenantID() string { return a.tenant }

type Store struct{ db *gorm.DB }

type key struct{}

func withTenant(ctx context.Context, actor Actor) context.Context {
	return context.WithValue(ctx, key{}, actor.TenantID())
}

func (s *Store) List(ctx context.Context, actor Actor) {
	ctx = withTenant(ctx, actor)
	tenant, _ := ctx.Value(key{}).(string)
	s.db.Where("tenant_id = ?", tenant).Find(&[]string{}) // want-flow
}
