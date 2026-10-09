package twincallbackdrops

import (
	"github.com/ThreeDotsLabs/watermill/message"
	"gorm.io/gorm"
)

type Service struct{ pub message.Publisher }

func (s *Service) Run(db *gorm.DB) error {
	return db.Transaction(func(tx *gorm.DB) error {
		_ = s.pub.Publish("topic")
		return nil
	})
}
