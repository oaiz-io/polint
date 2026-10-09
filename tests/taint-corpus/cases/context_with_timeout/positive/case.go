package positive

import (
	"context"
	"time"

	"gorm.io/gorm"
)

type Store struct{ db *gorm.DB }

func (s *Store) Find(ctx context.Context) {
	bounded, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	s.db.WithContext(bounded).Find(&[]string{}) // want-flow
}
