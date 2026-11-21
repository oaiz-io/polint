// @exports speak
function Cat() {} // @fn Cat
Cat.prototype.sound = function () { // @fn catSound
  return "meow";
};

function Dog() {} // @fn Dog
Dog.prototype.sound = function () { // @fn dogSound
  return "woof";
};

/** @param {Cat | Dog} pet */
function speak(pet) { // @fn speak
  return pet.sound(); // @calls catSound dogSound
}

module.exports = { speak: speak };

speak(new Cat()); // @calls speak Cat
speak(new Dog()); // @calls speak Dog
