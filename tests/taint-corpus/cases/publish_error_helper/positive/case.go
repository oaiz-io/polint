package positive

import (
	"fmt"

	"github.com/ThreeDotsLabs/watermill/message"
)

type Service struct{ pub message.Publisher }

func wrap(err error) error {
	if err == nil {
		return nil
	}
	return fmt.Errorf("wrapped: %w", err)
}

func (s *Service) Notify(topic string) error {
	return wrap(s.pub.Publish(topic)) // want-flow
}
