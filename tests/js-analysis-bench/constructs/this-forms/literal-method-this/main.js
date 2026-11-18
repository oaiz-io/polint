// @exports
const shape = {
  clone() { // @fn clone
    return { ...this };
  },
  copy() { // @fn copy
    return this.clone(); // @calls clone
  },
  later() { // @fn later
    const run = () => this.clone(); // @fn run @calls clone
    return run(); // @calls run
  },
};

shape.copy(); // @calls copy
shape.later(); // @calls later
