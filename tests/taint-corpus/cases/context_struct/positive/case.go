package positive

import (
	"context"

	"gorm.io/gorm"
)

type call struct {
	ctx context.Context
	db  *gorm.DB
}

func (c call) run() {
	c.db.WithContext(c.ctx).Find(&[]string{}) // want-flow
}

func Find(ctx context.Context, db *gorm.DB) { call{ctx: ctx, db: db}.run() }
