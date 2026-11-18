// @exports
function defaultHandler() { // @fn defaultHandler
  return "default";
}

class Base {
  handler = defaultHandler;

  constructor() { // @fn Base.constructor
    this.kind = "base";
  }

  describe() { // @fn baseDescribe
    return "base";
  }

  static create() { // @fn create
    return new this(); // @calls Base.constructor Derived.constructor
  }
}

class Derived extends Base {
  constructor() { // @fn Derived.constructor
    super(); // @calls Base.constructor
  }

  describe() { // @fn derivedDescribe
    return `derived ${super.describe()}`; // @calls baseDescribe
  }
}

const item = Derived.create(); // @calls create
item.describe(); // @calls derivedDescribe
item.handler(); // @calls defaultHandler
Base.create().describe(); // @calls create baseDescribe
