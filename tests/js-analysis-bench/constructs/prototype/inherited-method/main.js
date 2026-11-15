// @exports
function Animal(name) { // @fn Animal
  this.name = name;
}

Animal.prototype.describe = function describe() { // @fn describe
  return this.name + " says " + this.sound(); // @calls sound
};

function Dog(name) { // @fn Dog
  Animal.call(this, name); // @calls Animal
}

Dog.prototype = Object.create(Animal.prototype); // @external
Dog.prototype.sound = function sound() { // @fn sound
  return "woof";
};

const dog = new Dog("rex"); // @calls Dog
dog.describe(); // @calls describe
