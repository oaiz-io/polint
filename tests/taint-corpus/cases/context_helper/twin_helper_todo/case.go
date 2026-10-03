package twinhelpertodo

import (
	"context"

	"gorm.io/gorm"
)

type Store struct{ db *gorm.DB }

func (s *Store) scoped() *gorm.DB {
	return s.db.WithContext(context.TODO())
}

func (s *Store) Find(ctx context.Context) {
	_ = ctx
	s.scoped().Find(&[]string{})
}
