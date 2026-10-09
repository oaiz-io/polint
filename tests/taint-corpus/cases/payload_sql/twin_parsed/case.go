package twinparsed

import (
	"github.com/ThreeDotsLabs/watermill/message"
	"github.com/google/uuid"
	"gorm.io/gorm"
)

type Consumer struct{ db *gorm.DB }

func (c *Consumer) Handle(msg *message.Message) error {
	id, err := uuid.Parse(string(msg.Payload))
	if err != nil {
		return err
	}
	c.db.Exec("UPDATE items SET state = 'done' WHERE id = '" + id.String() + "'")
	return nil
}
