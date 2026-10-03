module example.com/taintcorpus

go 1.24

require (
	github.com/ThreeDotsLabs/watermill v1.0.0
	github.com/gin-gonic/gin v1.0.0
	github.com/google/uuid v1.0.0
	gorm.io/gorm v1.0.0
)

replace github.com/gin-gonic/gin => ./stubs/gin

replace gorm.io/gorm => ./stubs/gorm

replace github.com/ThreeDotsLabs/watermill => ./stubs/watermill

replace github.com/google/uuid => ./stubs/uuid
