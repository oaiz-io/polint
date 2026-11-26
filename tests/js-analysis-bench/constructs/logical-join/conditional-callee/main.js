// @exports
function onDone(result) { // @fn onDone
  return result;
}

function onFail(error) { // @fn onFail
  return error;
}

function log(message) { // @fn log
  return message;
}

function settle(ok, done, fail) { // @fn settle
  return (ok ? done : fail)(ok); // @calls onDone onFail
}

function notify(callback) { // @fn notify
  return callback && callback("notified"); // @calls onDone
}

function traced(handler) { // @fn traced
  return (log("trace"), handler)("traced"); // @calls log onFail
}

settle(true, onDone, onFail); // @calls settle
settle(false, onDone, onFail); // @calls settle
notify(onDone); // @calls notify
notify(null); // @calls notify
traced(onFail); // @calls traced
