// @exports
class Circle {
  area(): number { // @fn Circle.area
    return 3;
  }
}

class Square {
  area(): number { // @fn Square.area
    return 4;
  }
}

function measure(shape: Circle | Square): number { // @fn measure
  return shape.area(); // @calls Circle.area Square.area
}

measure(new Circle()); // @calls measure
measure(new Square()); // @calls measure
