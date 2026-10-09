package twinbackground

import (
	"context"

	"gorm.io/gorm"
)

type Store struct{ db *gorm.DB }

func (s *Store) Find(ctx context.Context, id string) {
	_ = ctx
	s.db.WithContext(context.Background()).Where("id = ?", id).Find(&[]string{})
}
