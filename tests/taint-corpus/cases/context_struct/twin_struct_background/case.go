package twinstructbackground

import (
	"context"

	"gorm.io/gorm"
)

type call struct {
	ctx context.Context
	db  *gorm.DB
}

func (c call) run() {
	c.db.WithContext(c.ctx).Find(&[]string{})
}

func Find(db *gorm.DB) { call{ctx: context.Background(), db: db}.run() }
