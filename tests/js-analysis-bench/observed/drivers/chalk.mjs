// Exercises chalk's ESM API: chained styles, colour models, nesting, levels
// and the subpath-imported helpers.
import chalk, { Chalk, supportsColor, modifiers, foregroundColors, backgroundColors } from "/bench/pkg/source/index.js";

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`chalk driver step ${name}: ${error.message}`);
  }
};

for (const level of [0, 1, 2, 3]) {
  step(`level ${level}`, () => {
    const instance = new Chalk({ level });
    instance.red("a");
    instance.bold.underline.bgBlue("b");
    instance.rgb(10, 20, 30)("c");
    instance.bgRgb(1, 2, 3).hex("#ff8800")("d");
    instance.ansi256(200).bgAnsi256(17)("e");
    instance.bgHex("#123")("f");
    instance.green(`outer ${instance.blue("inner")} outer`);
    instance.yellow("multi\nline\r\ntext");
    instance.visible("v");
    instance.reset.dim.italic.inverse.hidden.strikethrough.overline("g");
    instance.gray("h", "i", 3);
    instance.bold();
  });
}
step("default", () => {
  chalk.level = 3;
  chalk.cyan.bold("x");
  supportsColor;
  [modifiers, foregroundColors, backgroundColors].map((names) => names.length);
});
