// @exports
function onChange(value) { // @fn onChange
  return value;
}

class Store {
  set listener(fn) { // @fn setListener
    this._listener = fn;
  }

  get listener() { // @fn getListener
    return this._listener;
  }

  update(value) { // @fn update
    return this.listener(value); // @calls getListener onChange
  }
}

const store = new Store();
store.listener = onChange; // @calls setListener
store.update(1); // @calls update
