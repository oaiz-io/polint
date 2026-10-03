package twintimeoutbackground

import (
	"context"
	"time"

	"gorm.io/gorm"
)

type Store struct{ db *gorm.DB }

func (s *Store) Find(ctx context.Context) {
	_ = ctx
	bounded, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	s.db.WithContext(bounded).Find(&[]string{})
}
