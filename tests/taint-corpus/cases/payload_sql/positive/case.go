package positive

import (
	"github.com/ThreeDotsLabs/watermill/message"
	"gorm.io/gorm"
)

type Consumer struct{ db *gorm.DB }

func (c *Consumer) Handle(msg *message.Message) error {
	c.db.Exec("UPDATE items SET state = '" + string(msg.Payload) + "'") // want-flow
	return nil
}
