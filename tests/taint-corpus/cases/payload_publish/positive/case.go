package positive

import (
	"github.com/ThreeDotsLabs/watermill/message"
)

type Relay struct{ pub message.Publisher }

func (r *Relay) Handle(msg *message.Message) error {
	return r.pub.Publish("forwarded", message.NewMessage("id", msg.Payload)) // want-flow
}
