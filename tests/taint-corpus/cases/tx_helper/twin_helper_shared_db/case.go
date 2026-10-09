package twinhelpershareddb

import (
	"gorm.io/gorm"
)

type Repo struct{}

func (Repo) Append(db *gorm.DB, value string) error { return db.Create(value).Error }

type Service struct {
	db   *gorm.DB
	repo Repo
}

var shared *gorm.DB

func save(_ *gorm.DB, repo Repo, value string) error {
	return repo.Append(shared, value)
}

func (s *Service) Save(value string) error {
	return s.db.Transaction(func(tx *gorm.DB) error {
		return save(tx, s.repo, value)
	})
}
