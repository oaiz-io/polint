// Exercises the classic, mini and v3 zod APIs from TypeScript source: schema
// construction, parsing that succeeds and fails, transforms, error formatting,
// JSON Schema conversion and locales.
import * as z from "/bench/pkg/src/index.ts";
import * as zm from "/bench/pkg/src/mini/index.ts";
import * as z3 from "/bench/pkg/src/v3/index.ts";

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`zod driver step ${name}: ${error.message}`);
  }
};
const asyncStep = async (name, run) => {
  try {
    await run();
  } catch (error) {
    console.error(`zod driver step ${name}: ${error.message}`);
  }
};

const User = z.object({
  id: z.uuid(),
  name: z.string().min(1).max(40).trim(),
  email: z.email(),
  age: z.number().int().nonnegative().optional(),
  role: z.enum(["admin", "user"]).default("user"),
  tags: z.array(z.string()).max(3),
  site: z.url().nullable(),
  meta: z.record(z.string(), z.unknown()),
  when: z.iso.datetime(),
  kind: z.literal("user"),
});

const good = {
  id: "123e4567-e89b-42d3-a456-426614174000",
  name: " Ada ",
  email: "ada@example.com",
  age: 36,
  tags: ["a"],
  site: "https://example.com",
  meta: { a: 1 },
  when: "2020-01-01T00:00:00Z",
  kind: "user",
};
const bad = { id: "x", name: "", email: "no", age: -1.5, role: "root", tags: [1, 2, 3, 4], site: 4, meta: [], when: "yesterday", kind: "other" };

step("classic parse", () => User.parse(good));
step("classic failure", () => {
  const result = User.safeParse(bad);
  z.treeifyError(result.error);
  z.prettifyError(result.error);
  z.flattenError(result.error);
  result.error.format();
});
step("classic derived", () => {
  User.partial().parse({});
  User.pick({ id: true }).safeParse({ id: 1 });
  User.omit({ meta: true }).extend({ extra: z.boolean() }).safeParse(good);
  User.strict().safeParse({ ...good, extra: 1 });
  User.keyof().parse("id");
  User.required().safeParse(good);
});
step("classic composites", () => {
  const Shape = z.discriminatedUnion("type", [z.object({ type: z.literal("a"), a: z.string() }), z.object({ type: z.literal("b"), b: z.number() })]);
  Shape.parse({ type: "b", b: 1 });
  Shape.safeParse({ type: "c" });
  z.union([z.string(), z.number()]).parse(3);
  z.intersection(z.object({ a: z.string() }), z.object({ b: z.number() })).parse({ a: "x", b: 1 });
  z.tuple([z.string(), z.number()], z.boolean()).parse(["a", 1, true]);
  z.map(z.string(), z.number()).parse(new Map([["a", 1]]));
  z.set(z.number()).min(1).parse(new Set([1]));
  z.promise(z.number());
  z.lazy(() => z.string()).parse("x");
  z.bigint().positive().parse(1n);
  z.date().min(new Date(0)).parse(new Date(1000));
  z.nan().safeParse(1);
  z.file().safeParse(new File(["x"], "a.txt"));
  z.templateLiteral(["id-", z.number()]).parse("id-3");
  z.stringbool().parse("yes");
  z.json().parse({ a: [1, "b", null] });
});
step("classic transforms", () => {
  z.string().transform((value) => value.length).pipe(z.number().max(10)).parse("hello");
  z.string().refine((value) => value.startsWith("a"), { message: "must start with a" }).safeParse("b");
  z.object({ a: z.string(), b: z.string() }).superRefine((value, ctx) => {
    if (value.a !== value.b) ctx.addIssue({ code: "custom", message: "mismatch", path: ["b"] });
  }).safeParse({ a: "x", b: "y" });
  z.number().catch(0).parse("x");
  z.preprocess((value) => Number(value), z.number()).parse("4");
  z.coerce.number().parse("5");
  z.coerce.date().parse("2020-01-01");
  z.string().brand("Id").parse("x");
  z.string().readonly().parse("x");
  z.codec(z.iso.datetime(), z.date(), { decode: (text) => new Date(text), encode: (date) => date.toISOString() }).decode("2020-01-01T00:00:00Z");
});
await asyncStep("classic async", async () => {
  await z.string().refine(async (value) => value.length > 1).parseAsync("ab");
  await User.safeParseAsync(bad);
});
step("json schema", () => {
  z.toJSONSchema(User);
  z.toJSONSchema(z.union([z.string(), z.array(z.number())]), { target: "draft-7" });
  z.fromJSONSchema({ type: "object", properties: { a: { type: "string" } }, required: ["a"] }).parse({ a: "x" });
});
step("registry and locales", () => {
  const registry = z.registry();
  registry.add(User, { description: "user" });
  User.meta({ title: "User" });
  User.describe("a user");
  z.config(z.locales.de());
  User.safeParse(bad);
  z.config(z.locales.en());
});
step("mini", () => {
  const Mini = zm.object({ a: zm.string().check(zm.minLength(1)), b: zm.optional(zm.number()) });
  Mini.parse({ a: "x" });
  zm.safeParse(Mini, { a: "" });
  zm.array(zm.int()).parse([1]);
  zm.union([zm.string(), zm.boolean()]).parse(true);
  zm.pipe(zm.string(), zm.transform((value) => value.length)).parse("abc");
  zm.partial(Mini).parse({});
});
step("v3", () => {
  const Legacy = z3.object({ a: z3.string().email(), b: z3.number().min(1).optional(), c: z3.array(z3.enum(["x", "y"])) });
  Legacy.parse({ a: "a@b.co", c: ["x"] });
  const failure = Legacy.safeParse({ a: 1, b: 0, c: ["z"] });
  failure.error.flatten();
  failure.error.format();
  z3.union([z3.string(), z3.number()]).transform(String).parse(1);
  z3.discriminatedUnion("t", [z3.object({ t: z3.literal("a") }), z3.object({ t: z3.literal("b") })]).parse({ t: "a" });
  z3.string().refine((value) => value.length > 2).safeParse("a");
  z3.record(z3.number()).parse({ a: 1 });
  z3.date().safeParse("x");
  z3.lazy(() => z3.string()).parse("x");
});
