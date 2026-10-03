package twinnewpayload

import (
	"github.com/ThreeDotsLabs/watermill/message"
)

type Relay struct{ pub message.Publisher }

func (r *Relay) Handle(msg *message.Message) error {
	_ = msg
	return r.pub.Publish("forwarded", message.NewMessage("id", []byte("ping")))
}
