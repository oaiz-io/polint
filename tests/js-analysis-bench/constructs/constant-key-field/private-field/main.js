// @exports
function onTick() { // @fn onTick
  return "tick";
}

class Clock {
  #handler = onTick;

  tick() { // @fn tick
    return this.#handler(); // @calls onTick
  }
}

const clock = new Clock();
clock.tick(); // @calls tick
