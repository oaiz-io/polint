// @exports Job
const { EventEmitter } = require("events");

function emit(event) { // @fn emit
  return "local " + event;
}

class Job extends EventEmitter {
  finish(result) { // @fn finish
    return this.emit("done", result); // @callback onDone
  }
}

function onDone(result) { // @fn onDone
  return result;
}

const job = new Job(); // @external
job.on("done", onDone); // @external
job.finish(1); // @calls finish
emit("start"); // @calls emit

module.exports = { Job: Job };
