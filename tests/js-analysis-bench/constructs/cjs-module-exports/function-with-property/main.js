// @exports
const log = require("./logger"); // @external

log("ready"); // @calls log
log.format("direct"); // @calls format
