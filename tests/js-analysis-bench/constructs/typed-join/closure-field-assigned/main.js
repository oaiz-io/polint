// @exports Watcher
function Watcher() { // @fn Watcher
  this._onChange = function () {}; // @fn noop
}

Watcher.prototype.subscribe = function (fn) { // @fn subscribe
  this._onChange = fn;
  return this;
};

Watcher.prototype.notify = function (value) { // @fn notify
  return this._onChange(value); // @calls changed
};

function changed(value) { // @fn changed
  return value;
}

new Watcher().subscribe(changed).notify(1); // @calls Watcher subscribe notify

module.exports = { Watcher: Watcher };
