package positive

import (
	"github.com/ThreeDotsLabs/watermill/message"
	"gorm.io/gorm"
)

type Service struct{ pub message.Publisher }

func (s *Service) Run(db *gorm.DB) error {
	return db.Transaction(func(tx *gorm.DB) error {
		return s.pub.Publish("topic") // want-flow
	})
}
