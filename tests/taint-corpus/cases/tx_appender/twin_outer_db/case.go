package twinouterdb

import (
	"gorm.io/gorm"
)

type Repo struct{}

func (Repo) Append(db *gorm.DB, value string) error { return db.Create(value).Error }

type Service struct {
	db   *gorm.DB
	repo Repo
}

func (s *Service) Save(value string) error {
	return s.db.Transaction(func(tx *gorm.DB) error {
		return s.repo.Append(s.db, value)
	})
}
