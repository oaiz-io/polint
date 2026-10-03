package message

// Payload is a message body.
type Payload []byte

// Message is one message.
type Message struct {
	UUID     string
	Payload  Payload
	Metadata map[string]string
}

func NewMessage(uuid string, payload Payload) *Message {
	return &Message{UUID: uuid, Payload: payload}
}

// Publisher publishes messages.
type Publisher interface {
	Publish(topic string, messages ...*Message) error
	Close() error
}
