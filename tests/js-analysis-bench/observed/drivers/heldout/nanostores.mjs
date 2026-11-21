// Exercises nanostores as its readme and type declarations document it:
// atoms, maps and deep maps with get, set, setKey, listen and subscribe;
// computed and batched stores; batch and effect; key listeners; getKey,
// getPath, setPath and setByKey; the lifecycle events with keepMount and
// cleanStores; mapCreator; and the task API. Unmounting waits for
// STORE_UNMOUNT_DELAY on a real timer, which fires in a fixed order.
import {
  STORE_UNMOUNT_DELAY,
  allTasks,
  atom,
  batch,
  batched,
  cleanStores,
  cleanTasks,
  computed,
  deepMap,
  effect,
  getKey,
  getPath,
  keepMount,
  listenKeys,
  map,
  mapCreator,
  onMount,
  onNotify,
  onSet,
  onStart,
  onStop,
  readonlyType,
  setByKey,
  setPath,
  startTask,
  subscribeKeys,
  task,
} from "/bench/pkg/index.js";

const step = async (name, run) => {
  try {
    await run();
  } catch (error) {
    console.error(`nanostores driver step ${name}: ${error.message}`);
  }
};

const unmounted = () => new Promise((resolve) => setTimeout(resolve, STORE_UNMOUNT_DELAY + 20));

await step("atom", () => {
  const $counter = atom(0);
  const seen = [];
  const unbind = $counter.subscribe((value, oldValue) => seen.push([value, oldValue]));
  $counter.set($counter.get() + 1);
  $counter.set($counter.get() + 1);
  $counter.set(2);
  unbind();
  const unlisten = $counter.listen((value) => seen.push(value));
  $counter.set(3);
  unlisten();
  $counter.set(4);
  readonlyType($counter).get();
});

await step("map", () => {
  const $profile = map({ name: "anonymous" });
  const changes = [];
  const unbind = $profile.listen((profile, oldProfile, changed) => {
    if (changed) changes.push(`${changed} ${profile[changed]}`);
  });
  $profile.setKey("name", "Kazimir Malevich");
  $profile.setKey("email", "k@example.com");
  $profile.setKey("email", undefined);
  $profile.set({ name: "anonymous" });
  unbind();
  getKey($profile, "name");
});

await step("deep map", () => {
  const $state = deepMap({ user: { name: "Ada", tags: ["one"] }, items: [] });
  const unbind = $state.subscribe(() => {});
  $state.setKey("user.name", "Grace");
  $state.setKey("user.tags[1]", "two");
  $state.setKey("items[2]", { id: 3 });
  getKey($state, "user.tags[1]");
  unbind();
  getPath({ a: { b: { c: ["hey", "Hi!"] } } }, "a.b.c[1]");
  setPath({ a: { b: { c: [] } } }, "a.b.c[1]", "hey");
  setByKey({ a: { b: { c: [] } } }, ["a", "b", "c", 1], "hey");
});

await step("listen keys", () => {
  const $page = map({ blocked: false, title: "" });
  const seen = [];
  const unlisten = listenKeys($page, ["blocked"], (value, oldValue, changed) => seen.push(changed));
  const unsubscribe = subscribeKeys($page, ["title"], (value) => seen.push(value.title));
  $page.setKey("title", "Home");
  $page.setKey("blocked", true);
  $page.set({ blocked: false, title: "About" });
  unlisten();
  unsubscribe();
});

await step("computed", () => {
  const $users = atom([{ name: "a", isAdmin: true }, { name: "b", isAdmin: false }]);
  const $admins = computed($users, (users) => users.filter((user) => user.isAdmin));
  const $lastVisit = atom(1);
  const $posts = atom([{ publishedAt: 0 }, { publishedAt: 2 }]);
  const $newPosts = computed([$lastVisit, $posts], (lastVisit, posts) =>
    posts.filter((post) => post.publishedAt > lastVisit),
  );
  $admins.get();
  const unbind = $newPosts.subscribe(() => {});
  $users.set([...$users.get(), { name: "c", isAdmin: true }]);
  $admins.get();
  $lastVisit.set(0);
  unbind();
});

await step("batched", async () => {
  const $sortBy = atom("id");
  const $categoryId = atom("");
  const $link = batched([$sortBy, $categoryId], (sortBy, categoryId) =>
    `/api/entities?sortBy=${sortBy}&categoryId=${categoryId}`,
  );
  const seen = [];
  const unbind = $link.subscribe((link) => seen.push(link));
  $sortBy.set("date");
  $categoryId.set("1");
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  unbind();
});

await step("batch", () => {
  const $firstName = atom("");
  const $lastName = atom("");
  const $full = computed([$firstName, $lastName], (first, last) => `${first} ${last}`);
  const unbind = $full.subscribe(() => {});
  batch(() => {
    $firstName.set("Ada");
    $lastName.set("Lovelace");
  });
  unbind();
});

await step("effect", () => {
  const $enabled = atom(true);
  const $interval = atom(1000);
  const runs = [];
  const cancel = effect([$enabled, $interval], (enabled, interval) => {
    if (!enabled) return;
    runs.push(interval);
    return () => runs.push("cleanup");
  });
  $interval.set(500);
  $enabled.set(false);
  $enabled.set(true);
  cancel();
  const cancelOne = effect($interval, (interval) => {
    runs.push(interval);
  });
  $interval.set(250);
  cancelOne();
});

await step("events", async () => {
  const $store = atom(1);
  const events = [];
  const offMount = onMount($store, () => {
    events.push("mount");
    return () => events.push("unmount");
  });
  const offStart = onStart($store, () => events.push("start"));
  const offStop = onStop($store, () => events.push("stop"));
  const offSet = onSet($store, ({ newValue, abort }) => {
    if (newValue < 0) abort();
  });
  const offNotify = onNotify($store, ({ abort, oldValue }) => {
    if (oldValue === 100) abort();
  });
  const unbind = $store.listen(() => events.push("change"));
  $store.set(2);
  $store.set(-1);
  $store.set(100);
  $store.set(3);
  unbind();
  await unmounted();
  offMount();
  offStart();
  offStop();
  offSet();
  offNotify();

  const $profile = map({ name: "anonymous" });
  onSet($profile, ({ changed, abort }) => {
    if (changed === "locked") abort();
  });
  const unlisten = $profile.listen(() => {});
  $profile.setKey("locked", true);
  $profile.setKey("name", "Ada");
  unlisten();
});

await step("keep mount", async () => {
  const $profile = map({ name: "anonymous" });
  onMount($profile, () => {
    $profile.setKey("name", "loaded");
    return () => {};
  });
  keepMount($profile);
  $profile.get();
  cleanStores($profile);
  await unmounted();
});

await step("map creator", async () => {
  const User = mapCreator((store, id, name) => {
    store.set({ id, isLoading: true });
    const end = startTask();
    setTimeout(() => {
      store.set({ id, isLoading: false, name });
      end();
    }, 0);
    return () => {};
  });
  const user1 = User("1", "Ada");
  const user1Again = User("1", "Ada");
  const unbind = user1.subscribe(() => {});
  user1Again.get();
  await allTasks();
  User("2", "Grace").get();
  unbind();
  cleanStores(User);
  await unmounted();
});

await step("tasks", async () => {
  const $post = atom({ title: "" });
  onMount($post, () => {
    task(async () => {
      await Promise.resolve();
      $post.set({ title: "loaded" });
    });
  });
  $post.listen(() => {});
  await allTasks();
  const value = await task(() => 42);
  if (value !== 42) throw new Error("task did not return its value");
  await task(async () => {
    throw new Error("expected");
  }).catch(() => {});
  startTask();
  cleanTasks();
  await allTasks();
});
