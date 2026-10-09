package twinhelperdrops

import (
	"log"

	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func wrap(err error) error {
	log.Println(err)
	return nil
}

func (s *Service) Notify(topic string) error {
	return wrap(s.pub.Publish(topic))
}
