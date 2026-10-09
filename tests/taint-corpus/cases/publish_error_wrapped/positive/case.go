package positive

import (
	"fmt"

	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func (s *Service) Notify(topic string) error {
	if err := s.pub.Publish(topic); err != nil {
		return fmt.Errorf("notify: %w", err) // want-flow
	}
	return nil
}
