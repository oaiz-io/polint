package twinlogged

import (
	"log"

	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func (s *Service) Notify(topic string) error {
	if err := s.pub.Publish(topic); err != nil {
		log.Println(err)
	}
	return nil
}
