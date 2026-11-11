// @exports
const handlers = {};

handlers.start = function start() { // @fn start
  return "started";
};

handlers.stop = function stop() { // @fn stop
  return "stopped";
};

function run(table) { // @fn run
  table.start(); // @calls start
  return table.stop(); // @calls stop
}

run(handlers); // @calls run
