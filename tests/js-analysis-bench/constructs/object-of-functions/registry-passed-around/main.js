// @exports
const registry = {
  start() { // @fn start
    return "start";
  },
  stop() { // @fn stop
    return "stop";
  },
};

function control(ops) { // @fn control
  ops.start(); // @calls start
  return ops.stop(); // @calls stop
}

control(registry); // @calls control
