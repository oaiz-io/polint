package positive

import (
	"gorm.io/gorm"
)

type Repo struct{}

func (Repo) Append(db *gorm.DB, value string) error { return db.Create(value).Error }

type Service struct {
	db   *gorm.DB
	repo Repo
}

func save(tx *gorm.DB, repo Repo, value string) error {
	return repo.Append(tx, value) // want-flow
}

func (s *Service) Save(value string) error {
	return s.db.Transaction(func(tx *gorm.DB) error {
		return save(tx, s.repo, value)
	})
}
