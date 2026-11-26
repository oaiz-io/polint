// Exercises immer as its API documentation describes it: produce over objects
// and arrays, curried producers, returning a replacement or `nothing`,
// original, current, isDraft, isDraftable and isModified, freeze and
// setAutoFreeze, createDraft and finishDraft, immerable classes, a separate
// Immer instance, the patches plugin (produceWithPatches, patch listeners,
// applyPatches), the Map and Set plugin, and the array methods plugin.
import {
  Immer,
  applyPatches,
  castDraft,
  castImmutable,
  createDraft,
  current,
  enableArrayMethods,
  enableMapSet,
  enablePatches,
  finishDraft,
  freeze,
  immerable,
  isDraft,
  isDraftable,
  isModified,
  nothing,
  original,
  produce,
  produceWithPatches,
  setAutoFreeze,
  setUseStrictIteration,
  setUseStrictShallowCopy,
} from "/bench/pkg/src/immer.ts";

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`immer driver step ${name}: ${error.message}`);
  }
};

const baseState = [
  { title: "Learn TypeScript", done: true },
  { title: "Try Immer", done: false },
];

step("produce", () => {
  const next = produce(baseState, (draft) => {
    draft[1].done = true;
    draft.push({ title: "Tweet about it" });
  });
  const same = produce(baseState, () => {});
  if (same !== baseState || next === baseState) throw new Error("structural sharing");
});

step("curried", () => {
  const toggle = produce((draft, index) => {
    draft[index].done = !draft[index].done;
  });
  toggle(baseState, 0);
  const withInitial = produce((draft) => {
    draft.count++;
  }, { count: 0 });
  withInitial(undefined);
});

step("return", () => {
  produce({ user: { name: "a" } }, () => ({ replaced: true }));
  produce({ user: { name: "a" } }, () => nothing);
  produce([1, 2, 3], (draft) => draft.filter((value) => value > 1));
  produce({ nested: { deep: { value: 1 } } }, (draft) => {
    draft.nested.deep.value = 2;
    delete draft.nested.removed;
  });
});

step("inspect", () => {
  produce({ users: [{ name: "a" }], tags: ["x"] }, (draft) => {
    isDraft(draft);
    isDraftable(draft.users);
    isDraftable(new Date());
    draft.users[0].name = "b";
    isModified(draft);
    isModified(draft.tags);
    original(draft.users);
    current(draft);
    current(draft.users[0]);
  });
});

step("freeze", () => {
  freeze({ a: { b: 1 } }, true);
  freeze([1, 2]);
  setAutoFreeze(false);
  const unfrozen = produce({ a: 1 }, (draft) => {
    draft.a = 2;
  });
  unfrozen.a = 3;
  setAutoFreeze(true);
  setUseStrictShallowCopy(true);
  produce({ a: 1 }, (draft) => {
    draft.a = 2;
  });
  setUseStrictShallowCopy(false);
  setUseStrictIteration(true);
  produce({ a: { b: 1 } }, (draft) => {
    draft.a.b = 2;
  });
  setUseStrictIteration(false);
});

step("manual drafts", () => {
  const draft = createDraft({ todos: [{ title: "a" }] });
  draft.todos.push({ title: "b" });
  finishDraft(draft);
  castDraft({ a: 1 });
  castImmutable({ a: 1 });
});

step("classes", () => {
  class Clock {
    [immerable] = true;
    constructor(hour, minute) {
      this.hour = hour;
      this.minute = minute;
    }
    get time() {
      return `${this.hour}:${this.minute}`;
    }
    tick() {
      return produce(this, (draft) => {
        draft.minute++;
      });
    }
  }
  const clock = new Clock(12, 10);
  clock.tick().time;
});

step("instance", () => {
  const immer = new Immer({ autoFreeze: false, useStrictShallowCopy: false });
  immer.produce({ a: 1 }, (draft) => {
    draft.a = 2;
  });
  immer.setAutoFreeze(true);
  const draft = immer.createDraft({ b: [] });
  draft.b.push(1);
  immer.finishDraft(draft);
});

enablePatches();

step("patches", () => {
  const state = { name: "Micheal", age: 32, friends: ["a"] };
  const [next, patches, inversePatches] = produceWithPatches(state, (draft) => {
    draft.age = 33;
    draft.friends.push("b");
    draft.friends[0] = "c";
    delete draft.name;
  });
  applyPatches(next, inversePatches);
  applyPatches(state, patches);
  const collected = [];
  produce(
    { list: [1, 2, 3, 4] },
    (draft) => {
      draft.list.splice(1, 2);
      draft.list.unshift(0);
    },
    (forward, inverse) => collected.push(forward, inverse),
  );
  produce({ value: 1 }, () => nothing, (forward) => collected.push(forward));
  applyPatches({ a: { b: 1 } }, [
    { op: "replace", path: ["a", "b"], value: 2 },
    { op: "add", path: ["c"], value: [1] },
    { op: "add", path: ["c", "-"], value: 2 },
    { op: "remove", path: ["a"] },
  ]);
  applyPatches({ a: 1 }, [{ op: "replace", path: [], value: { b: 2 } }]);
});

enableMapSet();

step("map and set", () => {
  const state = { users: new Map([["a", { name: "a" }]]), tags: new Set(["x"]) };
  const next = produce(state, (draft) => {
    draft.users.set("b", { name: "b" });
    draft.users.get("a").name = "A";
    draft.users.delete("missing");
    for (const [, user] of draft.users) user.seen = true;
    draft.tags.add("y");
    draft.tags.delete("x");
    draft.tags.has("y");
    for (const tag of draft.tags.values()) tag.length;
  });
  const [, patches, inverse] = produceWithPatches(next, (draft) => {
    draft.users.clear();
    draft.tags.add("z");
  });
  applyPatches(next, patches);
  applyPatches(next, inverse);
  current(createDraft(state).users);
});

enableArrayMethods();

step("array methods", () => {
  const state = { items: [{ id: 1, done: false }, { id: 2, done: true }, { id: 3, done: false }] };
  produce(state, (draft) => {
    const open = draft.items.filter((item) => !item.done);
    open[0].done = true;
    draft.items.find((item) => item.id === 2).done = false;
    draft.items.findIndex((item) => item.id === 3);
    draft.items.some((item) => item.done);
    draft.items.every((item) => item.id > 0);
    draft.items.map((item) => item.id);
    draft.items.slice(1);
    draft.items.forEach((item) => {
      item.seen = true;
    });
    draft.items.reverse();
    draft.items.sort((left, right) => left.id - right.id);
    draft.items.pop();
    draft.items.shift();
  });
});
