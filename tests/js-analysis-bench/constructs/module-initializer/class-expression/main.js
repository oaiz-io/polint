// @exports Store
function read(key) { // @fn read
  return "value-" + key;
}

export const Store = class Store {
  constructor(prefix) { // @fn Store.constructor
    this.prefix = prefix;
  }

  get(key) { // @fn Store.get
    return this.prefix + read(key); // @calls read
  }

  getTwice(key) { // @fn Store.getTwice
    return this.get(key) + this.get(key); // @calls Store.get
  }
};

const store = new Store("v-"); // @calls Store.constructor
store.getTwice("a"); // @calls Store.getTwice
