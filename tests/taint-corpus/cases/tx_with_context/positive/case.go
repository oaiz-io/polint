package positive

import (
	"context"

	"gorm.io/gorm"
)

type Repo struct{}

func (Repo) Append(db *gorm.DB, value string) error { return db.Create(value).Error }

type Service struct {
	db   *gorm.DB
	repo Repo
}

func (s *Service) Save(ctx context.Context, value string) error {
	return s.db.Transaction(func(tx *gorm.DB) error {
		return s.repo.Append(tx.WithContext(ctx), value) // want-flow
	})
}
