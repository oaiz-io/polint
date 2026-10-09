package positive

import (
	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func (s *Service) Send(topic string) (int, error) {
	err := s.pub.Publish(topic)
	return 1, err // want-flow
}
