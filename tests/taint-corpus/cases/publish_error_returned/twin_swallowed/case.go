package twinswallowed

import (
	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func (s *Service) Notify(topic string, payload []byte) error {
	_ = s.pub.Publish(topic, message.NewMessage("id", payload))
	return nil
}
