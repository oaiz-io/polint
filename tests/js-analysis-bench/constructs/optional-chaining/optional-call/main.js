// @exports
function log(message) { // @fn log
  return message;
}

const logger = { log };
logger?.log("a"); // @calls log
const maybe = log;
maybe?.("b"); // @calls log
