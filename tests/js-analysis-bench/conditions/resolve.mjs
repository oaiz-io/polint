// Resolves every request of the condition-map corpus with Node's own
// resolver, under the conditions this process was started with (`-C`), and
// writes the file each request loads, or null where Node loads no file.
//
// Import mode asks `import.meta.resolve` with the importer as parent, which
// needs `--experimental-import-meta-resolve`; require mode asks the
// `require.resolve` of the importer.

import { statSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = process.env.CONDITIONS_ROOT;
const out = process.env.CONDITIONS_OUT;
const requests = JSON.parse(readFileSync(path.join(root, 'requests.json'), 'utf8'));

function loadedFile(absolute) {
  try {
    if (!statSync(absolute).isFile()) return null;
  } catch {
    return null;
  }
  return path.relative(root, absolute).split(path.sep).join('/');
}

function resolve(importer, specifier, mode) {
  const parent = path.join(root, importer);
  if (mode === 'import') {
    const url = import.meta.resolve(specifier, pathToFileURL(parent).href);
    return url.startsWith('file:') ? loadedFile(fileURLToPath(url)) : null;
  }
  const resolved = createRequire(parent).resolve(specifier);
  return path.isAbsolute(resolved) ? loadedFile(resolved) : null;
}

const picks = requests.map(([importer, specifier, mode]) => {
  try {
    return resolve(importer, specifier, mode);
  } catch (error) {
    if (typeof error?.code !== 'string') throw error;
    return null;
  }
});
writeFileSync(out, JSON.stringify({ node: process.version, picks }));
