// @exports Bus
const { EventEmitter } = require("events");

const logger = {
  emit(line) { // @fn loggerEmit
    return "log " + line;
  },
};

class Bus extends EventEmitter {
  send(message) { // @fn send
    logger.emit(message); // @calls loggerEmit
    return super.emit("message", message); // @callback receive
  }
}

function receive(message) { // @fn receive
  return message;
}

const bus = new Bus(); // @external
bus.on("message", receive); // @external
bus.send("hello"); // @calls send

module.exports = { Bus: Bus };
