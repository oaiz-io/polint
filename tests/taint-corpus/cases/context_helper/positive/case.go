package positive

import (
	"context"

	"gorm.io/gorm"
)

type Store struct{ db *gorm.DB }

func (s *Store) scoped(ctx context.Context) *gorm.DB {
	return s.db.WithContext(ctx) // want-flow
}

func (s *Store) Find(ctx context.Context) { s.scoped(ctx).Find(&[]string{}) }
