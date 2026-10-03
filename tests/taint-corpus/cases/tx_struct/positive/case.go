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

type writer struct{ db *gorm.DB }

func (w writer) write(value string) error {
	return Repo{}.Append(w.db, value) // want-flow
}

func (s *Service) Save(value string) error {
	return s.db.Transaction(func(tx *gorm.DB) error {
		return writer{db: tx}.write(value)
	})
}
