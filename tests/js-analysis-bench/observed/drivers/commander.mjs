// Exercises commander's CommonJS API: option and argument parsing, sub
// commands, help generation and error paths, without exiting the process.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { Command, Option, Argument, InvalidArgumentError } = require("/bench/pkg/index.js");

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    if (error.code === undefined) console.error(`commander driver step ${name}: ${error.message}`);
  }
};
const quiet = (command) => command.exitOverride().configureOutput({ writeOut: () => {}, writeErr: () => {}, outputError: () => {} });

step("options", () => {
  const program = quiet(new Command("tool"));
  program
    .version("1.0.0")
    .description("benchmark program")
    .option("-d, --debug", "debug output")
    .option("-n, --number <value>", "a number", (value) => {
      const parsed = Number(value);
      if (Number.isNaN(parsed)) throw new InvalidArgumentError("not a number");
      return parsed;
    }, 1)
    .option("--no-color", "disable colour")
    .option("-l, --list <items...>", "items")
    .addOption(new Option("-m, --mode <mode>").choices(["fast", "slow"]).default("fast").env("BENCH_MODE_UNSET"))
    .addOption(new Option("--secret").hideHelp())
    .argument("<input>", "input file")
    .addArgument(new Argument("[rest...]", "more").argOptional())
    .action(() => {});
  program.parse(["node", "tool", "-d", "-n", "3", "--list", "a", "b", "--mode", "slow", "in.txt", "x"]);
  program.opts();
  program.optsWithGlobals();
  program.helpInformation();
  program.args;
});

step("subcommands", () => {
  const program = quiet(new Command("git"));
  program.enablePositionalOptions().showSuggestionAfterError().showHelpAfterError();
  program.hook("preAction", () => {});
  program.command("clone <source> [destination]").alias("cl").option("--depth <n>").action(() => {});
  const remote = program.command("remote").description("manage remotes");
  remote.command("add <name> <url>").action(() => {});
  program.addCommand(quiet(new Command("status")).option("-s, --short").action(() => {}));
  program.parse(["clone", "repo", "dir", "--depth", "1"], { from: "user" });
  program.parse(["remote", "add", "origin", "url"], { from: "user" });
  program.parse(["status", "-s"], { from: "user" });
  program.commands.map((command) => command.helpInformation());
});

for (const argv of [["--number", "x"], ["--mode", "other"], ["--unknown"], ["clon"], ["--help"], ["--version"], []]) {
  step(`errors ${argv.join(" ")}`, () => {
    const program = quiet(new Command("tool"));
    program.version("1.0.0").option("--number <n>", "n", (value) => {
      throw new InvalidArgumentError(`bad ${value}`);
    });
    program.addOption(new Option("--mode <m>").choices(["a"]));
    program.command("clone").action(() => {});
    program.argument("<required>");
    program.parse(argv, { from: "user" });
  });
}

step("help configuration", () => {
  const program = quiet(new Command("styled"));
  program.configureHelp({ sortSubcommands: true, sortOptions: true, showGlobalOptions: true, helpWidth: 60 });
  program.option("-a, --alpha", "first").option("-b, --beta <x>", "second").command("sub").description("a sub command");
  program.addHelpText("after", "\nExample: styled -a");
  program.helpInformation();
  program.createHelp().formatHelp(program, program.createHelp());
});
