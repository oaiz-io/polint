package positive

import (
	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func (s *Service) Notify(topic string, payload []byte) error {
	err := s.pub.Publish(topic, message.NewMessage("id", payload))
	if err != nil {
		return err // want-flow
	}
	return nil
}
