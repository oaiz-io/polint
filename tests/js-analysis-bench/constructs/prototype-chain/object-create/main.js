// @exports
const animal = {
  speak() { // @fn speak
    return "...";
  },
};

const dog = Object.create(animal);
dog.bark = function bark() { // @fn bark
  return this.speak(); // @calls speak
};

const puppy = Object.create(dog);
puppy.bark(); // @calls bark
puppy.speak(); // @calls speak
