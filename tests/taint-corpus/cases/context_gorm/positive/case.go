package positive

import (
	"context"

	"gorm.io/gorm"
)

type Store struct{ db *gorm.DB }

func (s *Store) Find(ctx context.Context, id string) {
	s.db.WithContext(ctx).Where("id = ?", id).Find(&[]string{}) // want-flow
}
