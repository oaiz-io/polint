// @exports
function speak() { // @fn speak
  return "speak";
}

function Wrapper() { // @fn Wrapper
  return { speak };
}

function makeClass() { // @fn makeClass
  function Made() {} // @fn Made
  Made.prototype.hello = function hello() { // @fn hello
    return "hello";
  };
  return Made;
}

const wrapped = new Wrapper(); // @calls Wrapper
wrapped.speak(); // @calls speak
const Made = makeClass(); // @calls makeClass
new Made().hello(); // @calls Made hello
