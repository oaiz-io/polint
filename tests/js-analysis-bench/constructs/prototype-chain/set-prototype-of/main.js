// @exports
function Shape() {} // @fn Shape

Shape.prototype = {
  area() { // @fn area
    return 0;
  },
};

const base = {
  name() { // @fn name
    return "base";
  },
};

const square = new Shape(); // @calls Shape
square.area(); // @calls area
const named = {};
Object.setPrototypeOf(named, base);
named.name(); // @calls name
