// @exports
async function fetchUser(id) { // @fn fetchUser
  return { id };
}

const fetchRole = async (user) => (user.id === 1 ? "admin" : "user"); // @fn fetchRole

async function profile(id) { // @fn profile
  const user = await fetchUser(id); // @calls fetchUser
  const role = await fetchRole(user); // @calls fetchRole
  return { user, role };
}

await profile(1); // @calls profile
