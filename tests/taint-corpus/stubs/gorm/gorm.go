package gorm

import "context"

// DB is a database handle.
type DB struct{ Error error }

func (db *DB) Raw(sql string, values ...any) *DB       { return db }
func (db *DB) Exec(sql string, values ...any) *DB      { return db }
func (db *DB) Where(query any, args ...any) *DB        { return db }
func (db *DB) Order(value any) *DB                     { return db }
func (db *DB) Find(dest any, conds ...any) *DB         { return db }
func (db *DB) First(dest any, conds ...any) *DB        { return db }
func (db *DB) Create(value any) *DB                    { return db }
func (db *DB) Model(value any) *DB                     { return db }
func (db *DB) WithContext(ctx context.Context) *DB     { return db }
func (db *DB) Scopes(funcs ...func(*DB) *DB) *DB       { return db }
func (db *DB) Transaction(fc func(tx *DB) error) error { return fc(db) }
