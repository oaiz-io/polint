// @exports
function connect() { // @fn connect
  return "connected";
}

function retry() { // @fn retry
  return "retried";
}

function close() { // @fn close
  return "closed";
}

function start({ open = connect, again: onRetry, ...rest }) { // @fn start
  open(); // @calls connect
  onRetry(); // @calls retry
  return rest.finish(); // @calls close
}

start({ again: retry, finish: close }); // @calls start
