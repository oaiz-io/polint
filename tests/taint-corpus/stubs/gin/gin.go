package gin

import "net/http"

// Context is the request context a handler gets.
type Context struct {
	Request *http.Request
	Writer  http.ResponseWriter
}

func (c *Context) Param(key string) string               { return "" }
func (c *Context) Query(key string) string               { return "" }
func (c *Context) DefaultQuery(key, value string) string { return value }
func (c *Context) PostForm(key string) string            { return "" }
func (c *Context) GetHeader(key string) string           { return "" }
func (c *Context) ShouldBindJSON(obj any) error          { return nil }
func (c *Context) BindJSON(obj any) error                { return nil }
func (c *Context) JSON(code int, obj any)                {}
func (c *Context) AbortWithStatus(code int)              {}
