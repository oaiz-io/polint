package positive

import (
	"encoding/json"
	"os/exec"

	"github.com/ThreeDotsLabs/watermill/message"
)

type event struct{ Command string }

func Handle(msg *message.Message) error {
	var evt event
	if err := json.Unmarshal(msg.Payload, &evt); err != nil {
		return err
	}
	return exec.Command(evt.Command).Run() // want-flow
}
