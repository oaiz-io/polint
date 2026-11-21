// @exports
function render(value) { // @fn render
  return String(value);
}

function format(value) { // @fn format
  return `[${value}]`;
}

const defaults = { render, format };
const options = { ...defaults, format: render };
options.render(1); // @calls render
options.format(2); // @calls render
const pipeline = { steps: [format, ...[render]] };
pipeline.steps[1](3); // @calls render
