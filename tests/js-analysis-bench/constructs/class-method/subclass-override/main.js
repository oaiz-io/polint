// @exports
import { Shape } from "./shape.js";

class Square extends Shape {
  constructor(side) { // @fn Square.constructor
    super(); // @external
    this.side = side;
  }

  area() { // @fn Square.area
    return this.side * this.side + super.area(); // @calls Shape.area
  }
}

new Square(2).describe(); // @calls Square.constructor Shape.describe
