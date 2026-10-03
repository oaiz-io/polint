package twinnil

import (
	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func (s *Service) Send(topic string) (int, error) {
	_ = s.pub.Publish(topic)
	return 1, nil
}
