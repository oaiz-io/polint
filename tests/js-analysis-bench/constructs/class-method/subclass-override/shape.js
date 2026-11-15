// @exports Shape
export class Shape {
  describe() { // @fn Shape.describe
    return "area " + this.area(); // @calls Square.area
  }

  area() { // @fn Shape.area
    return 0;
  }
}
