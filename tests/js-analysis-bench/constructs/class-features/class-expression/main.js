// @exports
function makeBase() { // @fn makeBase
  return class {
    constructor() { // @fn AnonymousBase.constructor
      this.ready = true;
    }

    greet() { // @fn greet
      return "hi";
    }
  };
}

const Parent = makeBase(); // @calls makeBase

class Child extends Parent { // @calls AnonymousBase.constructor
  run() { // @fn run
    return this.greet(); // @calls greet
  }
}

new Child().run(); // @calls run
