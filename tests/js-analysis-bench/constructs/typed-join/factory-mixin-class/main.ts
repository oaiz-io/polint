// @exports
class Plain {
  value: number;

  constructor() { // @fn Plain.constructor
    this.value = 0;
  }
}

type Ctor = new (...args: any[]) => object;

function withLogging<Base extends Ctor>(base: Base) { // @fn withLogging
  return class Logged extends base {
    constructor(...args: any[]) { // @fn Logged.constructor
      super(...args); // @calls Plain.constructor
    }
  };
}

const Service = withLogging(Plain); // @calls withLogging
new Service(); // @calls Logged.constructor
